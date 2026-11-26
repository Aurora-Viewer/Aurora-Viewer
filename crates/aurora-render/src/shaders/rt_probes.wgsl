// The reflection probes at a ray's hit (rt_reflect.wgsl): the ambient and
// the reflection the shading gives a surface with probes (common.wgsl
// `shade_pbr`: their irradiance along its normal, their radiance along
// what it mirrors). A hit the view does not show is shaded by the ray
// tracer, and with the sky's hemisphere for both it took the open sky's
// light inside a room: a metal's reflection of what stands behind the
// camera came out in the sky's colour.
//
// Port of common.wgsl `sample_probes_ex`, the walk of the whole list (a hit
// is no fragment of the view: it has no screen tile), without water's case.
// Each shader that takes it declares `probe_set` (probes.rs `Probes::buffer`),
// `probe_tex`, `probe_sampler` and `probe_irr_tex`. The structs are
// common.wgsl's, word for word (a test of rt_shadows.rs compares them).

// probes: structs
const MAX_PROBES: u32 = 256u;
// A set of probes: bit i of word i / 32 for probe i of the list
// (probes::ProbeSets; the lights' sets hold 64 and are two words).
struct ProbeMask {
    w: array<u32, 8>,
};
struct Probe {
    center_radius: vec4<f32>,
    // x ambiance scale, y radiance scale, z fade in, w kind (0 auto, 1 sphere, -1 box)
    params: vec4<f32>,
    slot: vec4<u32>,
    box_inv: mat4x4<f32>,
};
struct ProbeSet {
    count: u32,
    max_lod: f32,
    enabled: u32,
    _pad: u32,
    // the box probes, which sample_probes_pair looks through first
    boxes: ProbeMask,
    // the probes that cover each screen column, each screen row and each
    // slice of depth of the main view (probes::tiles): one set per element
    tile_grid: vec4<f32>,       // x, y: columns and rows per pixel
    tile_forward: vec4<f32>,    // camera forward axis (fragment depth)
    tile_mode: vec4<u32>,
    tile_columns: array<ProbeMask, 32>,
    tile_rows: array<ProbeMask, 32>,
    tile_slices: array<ProbeMask, 32>,
    probes: array<Probe, MAX_PROBES>,
};
// probes: end

// What the probes give at `pos` along `dir`: their radiance at mip `lod`,
// or with `irradiance` their irradiance, each probe's mixed with `amblit`
// by its ambiance. w 0 when no probe covers `pos`.
fn probes_along(pos: vec3<f32>, dir: vec3<f32>, lod: f32, irradiance: bool, amblit: vec3<f32>) -> vec4<f32> {
    let count = min(probe_set.count, MAX_PROBES);
    // never let automatic probes encroach on box probes
    var sample_automatic = true;
    for (var i = 1u; i < count; i++) {
        if (probe_set.probes[i].params.w < 0.0) {
            let lp = (probe_set.probes[i].box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (all(abs(lp) <= vec3<f32>(1.0))) {
                sample_automatic = false;
            }
        }
    }
    var col0 = vec3<f32>(0.0);
    var col1 = vec3<f32>(0.0);
    var w0 = 0.0;
    var w1 = 0.0;
    var dw1 = 0.0;
    for (var i = 0u; i < count; i++) {
        let kind = probe_set.probes[i].params.w;
        let manual = kind != 0.0;
        if (!manual && !sample_automatic) {
            continue;
        }
        let sphere = probe_set.probes[i].center_radius;
        let scale = probe_set.probes[i].params;
        let c = sphere.xyz;
        var v: vec3<f32>;
        var w: f32;
        var dw = 0.0;
        if (kind < 0.0) {
            // box influence volume with parallax correction
            let box_inv = probe_set.probes[i].box_inv;
            let lp = (box_inv * vec4<f32>(pos, 1.0)).xyz;
            if (any(abs(lp) > vec3<f32>(1.0))) {
                continue;
            }
            let ray = (box_inv * vec4<f32>(dir, 0.0)).xyz;
            // the irradiance lookup pads the box (boxIntersect scale 3)
            let pad = select(1.0, 3.0, irradiance);
            let first = (vec3<f32>(pad) - lp) / ray;
            let second = (vec3<f32>(-pad) - lp) / ray;
            let far = max(first, second);
            let dist = min(far.x, min(far.y, far.z));
            v = pos + dir * dist - c;
            w = max(1.0 - max(max(abs(lp.x), abs(lp.y)), abs(lp.z)), 0.001);
        } else {
            let r = sphere.w;
            let delta = pos - c;
            if (i != 0u && dot(delta, delta) > r * r) {
                continue;
            }
            if (kind >= 1.0) {
                // sphereIntersect: parallax for manual sphere probes only
                let l = c - pos;
                let tca = dot(l, dir);
                let d2 = dot(l, l) - tca * tca;
                v = pos + dir * (tca + sqrt(max(r * r - d2, 0.0))) - c;
            } else {
                v = dir;
            }
            // sphereWeight
            let r1 = r * 0.5;
            let d = max(length(delta), 0.001);
            let atten = 1.0 - max(d - r1, 0.0) / max(r - r1, 0.001);
            let base = scale.z / d;
            dw = base * atten * max(r, 1.0) * 4.0;
            w = base * atten;
        }
        let slot = i32(probe_set.probes[i].slot.x);
        var refcol: vec3<f32>;
        if (irradiance) {
            let irr = textureSampleLevel(probe_irr_tex, probe_sampler, v, slot, 0.0).rgb * scale.x;
            refcol = mix(amblit, irr, min(scale.x, 1.0));
        } else {
            refcol = textureSampleLevel(probe_tex, probe_sampler, v, slot, lod).rgb * scale.y;
        }
        if (manual) {
            col1 += refcol * w;
            w1 += w;
            dw1 += dw;
        } else {
            col0 += refcol * w;
            w0 += w;
        }
    }
    // mix automatic and manual probes
    if (sample_automatic && w0 > 0.0) {
        col0 /= w0;
        if (w1 > 0.0) {
            col1 = mix(col0, col1 / w1, min(dw1, 1.0));
            col0 = vec3<f32>(0.0);
        }
    } else if (w1 > 0.0) {
        col1 /= w1;
        col0 = vec3<f32>(0.0);
    }
    return vec4<f32>(col0 + col1, select(0.0, 1.0, w0 + w1 > 0.0));
}

// The ambient a surface at `p` takes along its normal `n`: the probes'
// where one covers it (`shade_pbr`'s `amb`), the sky's hemisphere without
// probes.
fn ambient_at(lit: HitLight, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let sky = hemisphere(lit, n);
    if (probe_set.enabled == 0u) {
        return sky;
    }
    let s = probes_along(p, n, 0.0, true, sky);
    return select(sky, s.rgb, s.w > 0.0);
}

// What a surface of this roughness at `p` mirrors along `r` without a ray:
// the probes' radiance (`shade_pbr`'s `env`), the sky's hemisphere without
// probes.
fn mirrored_at(lit: HitLight, p: vec3<f32>, r: vec3<f32>, roughness: f32) -> vec3<f32> {
    let sky = hemisphere(lit, r);
    if (probe_set.enabled == 0u) {
        return sky;
    }
    let s = probes_along(p, r, clamp(roughness, 0.0, 1.0) * probe_set.max_lod, false, vec3<f32>(0.0));
    return select(sky, s.rgb, s.w > 0.0);
}
