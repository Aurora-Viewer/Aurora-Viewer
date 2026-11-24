// The update of the traced global illumination's probes (gi.rs), after
// gi_common.wgsl. Each frame, in this order:
// - cs_reset: the probes whose cell entered the grid as it scrolled lose
//   their light (their state goes back to 0: invalid, age 0);
// - cs_blend_depth, cs_blend_irradiance: one workgroup per probe of the
//   frame's list folds its rays (radiance and distance, written by the
//   tracer) into each interior texel of its tiles, with a hysteresis that
//   grows with the probe's age, and writes the texel with the border texels
//   that copy it into the atlas the shading samples;
// - cs_settle: the updated probes are valid and one update older, and
//   move out of the walls and away from the faces they are too near
//   (Majercik et al. 2021, sections 4.1 and 4.2: relocation and the
//   probes inside geometry, from the update's fixed rays).
// Majercik et al., "Dynamic Diffuse Global Illumination with Ray-Traced
// Irradiance Fields" (JCGT 2019), sections 4.1 to 4.3, written from the
// paper.

@group(0) @binding(0) var<uniform> gi: GiUniforms;
// GI_RAYS per updated probe, in the order of the frame's list: rgb the
// radiance reaching the probe along the ray, w the distance to the hit
// (negative: the ray left the scene and rgb is the sky's)
@group(0) @binding(1) var<storage, read> gi_rays: array<vec4<f32>>;
// the light of every interior texel as last blended, two f16 pairs
@group(0) @binding(2) var<storage, read_write> gi_irr_hist: array<vec2<u32>>;
@group(0) @binding(3) var<storage, read_write> gi_depth_hist: array<vec2<u32>>;
@group(0) @binding(4) var<storage, read_write> gi_state: array<u32>;
@group(0) @binding(5) var gi_irr_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var gi_depth_out: texture_storage_2d<rgba16float, write>;
// each probe's move from its grid point (xyz, m)
@group(0) @binding(7) var<storage, read_write> gi_offsets: array<vec4<f32>>;

// The share of the old light a probe keeps: none on its first update, then
// more at each one, up to GI_MAX_HYSTERESIS (gi.rs `hysteresis`).
fn gi_hysteresis(state: u32) -> f32 {
    if ((state & GI_VALID) == 0u) {
        return 0.0;
    }
    let age = f32(state & GI_AGE);
    return min(1.0 - 1.0 / (age + 1.0), GI_MAX_HYSTERESIS);
}

// A ray's light as the blend takes it: a value that is not a number, or
// past what a 16-bit float holds, counts as none, so that one bad ray
// never poisons a probe's history for good.
fn gi_ray_light(i: u32) -> vec3<f32> {
    let c = gi_rays[i].rgb;
    let finite = all(c == c) && all(abs(c) < vec3<f32>(6.0e4));
    return select(vec3<f32>(0.0), max(c, vec3<f32>(0.0)), finite);
}

// A texel's light after an update (gi.rs `blend`, its twin): a change
// past GI_CHANGE drops the hysteresis by 0.75, so that a lamp turned on or
// off shows within a few updates; a rise past GI_JUMP is taken a quarter
// at a time (RTXGI's ProbeBlendingCS, from the 2021 paper).
fn gi_blend(old: vec3<f32>, fresh: vec3<f32>, state: u32) -> vec3<f32> {
    var h = gi_hysteresis(state);
    var f = fresh;
    if ((state & GI_VALID) != 0u) {
        let delta = fresh - old;
        let change = abs(delta);
        if (max(change.x, max(change.y, change.z)) > GI_CHANGE) {
            h = max(h - 0.75, 0.0);
        }
        if (max(delta.x, max(delta.y, delta.z)) > GI_JUMP) {
            f = old + delta * 0.25;
        }
    }
    return mix(f, old, h);
}

// A texel's sight of the sky after an update (gi.rs `blend_sky`, its
// twin): as `gi_blend`, a change past GI_CHANGE drops the hysteresis.
fn gi_blend_sky(old: f32, fresh: f32, state: u32) -> f32 {
    var h = gi_hysteresis(state);
    if ((state & GI_VALID) != 0u && abs(fresh - old) > GI_CHANGE) {
        h = max(h - 0.75, 0.0);
    }
    return mix(fresh, old, h);
}

fn gi_pack(v: vec4<f32>) -> vec2<u32> {
    return vec2<u32>(pack2x16float(v.xy), pack2x16float(v.zw));
}

fn gi_unpack(p: vec2<u32>) -> vec4<f32> {
    return vec4<f32>(unpack2x16float(p.x), unpack2x16float(p.y));
}

// Writes interior texel `t` of a tile at `base` (its corner, border
// included), and the border texels that copy it: the octahedral square
// folds along its edges, so a border texel repeats the interior texel
// across the edge, mirrored (gi.rs `border_source`, its inverse).
fn gi_store_irr(base: vec2<u32>, t: vec2<u32>, v: vec4<f32>) {
    let n = GI_IRR_INTERIOR;
    let s = n + 1u;
    textureStore(gi_irr_out, base + t + vec2<u32>(1u), v);
    if (t.x == 0u) { textureStore(gi_irr_out, base + vec2<u32>(0u, s - t.y - 1u), v); }
    if (t.x == n - 1u) { textureStore(gi_irr_out, base + vec2<u32>(s, s - t.y - 1u), v); }
    if (t.y == 0u) { textureStore(gi_irr_out, base + vec2<u32>(s - t.x - 1u, 0u), v); }
    if (t.y == n - 1u) { textureStore(gi_irr_out, base + vec2<u32>(s - t.x - 1u, s), v); }
    if (t.x == n - 1u && t.y == n - 1u) { textureStore(gi_irr_out, base, v); }
    if (t.x == 0u && t.y == n - 1u) { textureStore(gi_irr_out, base + vec2<u32>(s, 0u), v); }
    if (t.x == n - 1u && t.y == 0u) { textureStore(gi_irr_out, base + vec2<u32>(0u, s), v); }
    if (t.x == 0u && t.y == 0u) { textureStore(gi_irr_out, base + vec2<u32>(s, s), v); }
}

fn gi_store_depth(base: vec2<u32>, t: vec2<u32>, v: vec4<f32>) {
    let n = GI_DEPTH_INTERIOR;
    let s = n + 1u;
    textureStore(gi_depth_out, base + t + vec2<u32>(1u), v);
    if (t.x == 0u) { textureStore(gi_depth_out, base + vec2<u32>(0u, s - t.y - 1u), v); }
    if (t.x == n - 1u) { textureStore(gi_depth_out, base + vec2<u32>(s, s - t.y - 1u), v); }
    if (t.y == 0u) { textureStore(gi_depth_out, base + vec2<u32>(s - t.x - 1u, 0u), v); }
    if (t.y == n - 1u) { textureStore(gi_depth_out, base + vec2<u32>(s - t.x - 1u, s), v); }
    if (t.x == n - 1u && t.y == n - 1u) { textureStore(gi_depth_out, base, v); }
    if (t.x == 0u && t.y == n - 1u) { textureStore(gi_depth_out, base + vec2<u32>(s, 0u), v); }
    if (t.x == n - 1u && t.y == 0u) { textureStore(gi_depth_out, base + vec2<u32>(0u, s), v); }
    if (t.x == 0u && t.y == 0u) { textureStore(gi_depth_out, base + vec2<u32>(s, s), v); }
}

@compute @workgroup_size(64)
fn cs_reset(@builtin(global_invocation_id) id: vec3<u32>) {
    let total = gi.dims.w * GI_CASCADES;
    if (id.x >= total) {
        return;
    }
    let c = id.x / gi.dims.w;
    let s = gi_slot(id.x % gi.dims.w);
    let cell = gi_cell_of_slot(s, gi.cascades[c].origin.xyz);
    let old = gi.cascades[c].old_origin.xyz;
    let d = vec3<i32>(gi.dims.xyz);
    if (any(cell < old) || any(cell >= old + d)) {
        gi_state[id.x] = 0u;
        gi_offsets[id.x] = vec4<f32>(0.0);
    }
}

@compute @workgroup_size(GI_IRR_INTERIOR, GI_IRR_INTERIOR)
fn cs_blend_irradiance(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let k = wg.x;
    let probe = gi_update_probe(k);
    if (probe.x == GI_NONE) {
        return;
    }
    let dir = gi_texel_dir(lid.xy, GI_IRR_INTERIOR);
    var sum = vec3<f32>(0.0);
    var sky = 0.0;
    var weight = 0.0;
    // (the fixed rays are not shaded; a ray that met a face's back brings
    // no light: it counts for nothing rather than as dark, as in RTXGI)
    for (var r = GI_FIXED_RAYS; r < GI_RAYS; r++) {
        let t = gi_rays[k * GI_RAYS + r].w;
        if (gi_is_back(t)) {
            continue;
        }
        let w = max(dot(dir, gi_ray_dir(r)), 0.0);
        sum += gi_ray_light(k * GI_RAYS + r) * w;
        sky += select(0.0, w, gi_is_miss(t));
        weight += w;
    }
    let fresh = sum / max(weight, 1e-4);
    // a: the share of the texel's cosine lobe whose rays met nothing (the
    // sky), for the shading's sky ambient (gi_sample.wgsl `gi_sky`)
    let fresh_sky = sky / max(weight, 1e-4);
    let global = probe.x * gi.dims.w + probe.y;
    let slot = gi_slot(probe.y);
    let texel = global * GI_IRR_INTERIOR * GI_IRR_INTERIOR + lid.y * GI_IRR_INTERIOR + lid.x;
    let old = gi_unpack(gi_irr_hist[texel]);
    let state = gi_state[global];
    let v = vec4<f32>(gi_blend(old.rgb, fresh, state), gi_blend_sky(old.a, fresh_sky, state));
    gi_irr_hist[texel] = gi_pack(v);
    gi_store_irr(gi_tile_origin(probe.x, slot, GI_IRR_TILE), lid.xy, v);
}

@compute @workgroup_size(GI_DEPTH_INTERIOR, GI_DEPTH_INTERIOR)
fn cs_blend_depth(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_id) lid: vec3<u32>) {
    let k = wg.x;
    let probe = gi_update_probe(k);
    if (probe.x == GI_NONE) {
        return;
    }
    let far = gi.cascades[probe.x].spacing.z;
    let dir = gi_texel_dir(lid.xy, GI_DEPTH_INTERIOR);
    var sum = vec2<f32>(0.0);
    var weight = 0.0;
    for (var r = GI_FIXED_RAYS; r < GI_RAYS; r++) {
        // a sharp lobe: the distance seen along the texel's direction
        // (the paper's depth sharpness of 50); a face's back counts at a
        // fifth of its distance, so that what lies past it is hidden
        let w = pow(max(dot(dir, gi_ray_dir(r)), 0.0), GI_DEPTH_SHARPNESS);
        let t = gi_rays[k * GI_RAYS + r].w;
        let dist = select(min(abs(t), far), far, gi_is_miss(t));
        sum += vec2<f32>(dist, dist * dist) * w;
        weight += w;
    }
    // a texel no ray came near keeps the far distance: it hides nothing
    let fresh = select(vec2<f32>(far, far * far), sum / weight, weight > 1e-4);
    let global = probe.x * gi.dims.w + probe.y;
    let slot = gi_slot(probe.y);
    let texel = global * GI_DEPTH_INTERIOR * GI_DEPTH_INTERIOR + lid.y * GI_DEPTH_INTERIOR + lid.x;
    let h = gi_hysteresis(gi_state[global]);
    let v = vec4<f32>(mix(fresh, gi_unpack(gi_depth_hist[texel]).xy, h), 0.0, 1.0);
    gi_depth_hist[texel] = gi_pack(v);
    gi_store_depth(gi_tile_origin(probe.x, slot, GI_DEPTH_TILE), lid.xy, v);
}

// A probe's new offset (xyz) and whether it is inside a wall (w), from
// its fixed rays (gi.rs `relocate`, the CPU twin: RTXGI's
// ProbeRelocationCS, written from the 2021 paper).
fn gi_relocate(k: u32, offset: vec3<f32>, spacing: f32) -> vec4<f32> {
    let far = 1.0e6;
    var backfaces = 0u;
    var closest_back = vec4<f32>(0.0, 0.0, 0.0, far);
    var closest_front = vec4<f32>(0.0, 0.0, 0.0, far);
    var farthest_front = vec4<f32>(0.0);
    for (var r = 0u; r < GI_FIXED_RAYS; r++) {
        let w = gi_rays[k * GI_RAYS + r].w;
        let dir = gi_fibonacci(r, GI_FIXED_RAYS);
        if (gi_is_back(w)) {
            backfaces += 1u;
            let d = -w / GI_BACK_SCALE;
            if (d < closest_back.w) {
                closest_back = vec4<f32>(dir, d);
            }
            continue;
        }
        let d = select(w, far, w < 0.0);
        if (d < closest_front.w) {
            closest_front = vec4<f32>(dir, d);
        }
        if (d > farthest_front.w) {
            farthest_front = vec4<f32>(dir, d);
        }
    }
    let min_front = GI_MIN_FRONT * spacing;
    let inside = f32(backfaces) > GI_BACKFACE_RATIO * f32(GI_FIXED_RAYS);
    var full = offset;
    if (inside) {
        // out through the nearest back face
        full = offset + closest_back.xyz * (closest_back.w + min_front * 0.5);
    } else if (closest_front.w < min_front) {
        // away from the face it is too near, never through the farthest
        if (dot(closest_front.xyz, farthest_front.xyz) <= 0.0) {
            full = offset + farthest_front.xyz * min(farthest_front.w, 0.25 * spacing);
        }
    } else {
        // nothing near: back towards its grid point
        let len = length(offset);
        if (len > 1e-5) {
            full = offset - offset / len * min(closest_front.w - min_front, len);
        }
    }
    let n = full / spacing;
    let kept = select(offset, full, dot(n, n) < GI_MAX_OFFSET * GI_MAX_OFFSET);
    return vec4<f32>(kept, select(0.0, 1.0, inside));
}

// How open the place of the probe of update `k` is, from its fixed rays,
// which go every way alike: twice the share of them that met nothing, at
// most 1. Out in the open the sky is half of every way (1), in a closed
// room none (0), under a porch's roof the ways under its eaves. The read
// of the default reflection probe takes it (common.wgsl
// `default_probe_outside`): unlike the sky's share along a normal, it does
// not fall on a face turned to the ground out in the open.
fn gi_openness(k: u32) -> f32 {
    var misses = 0u;
    for (var r = 0u; r < GI_FIXED_RAYS; r++) {
        if (gi_is_miss(gi_rays[k * GI_RAYS + r].w)) {
            misses += 1u;
        }
    }
    return min(2.0 * f32(misses) / f32(GI_FIXED_RAYS), 1.0);
}

@compute @workgroup_size(64)
fn cs_settle(@builtin(global_invocation_id) id: vec3<u32>) {
    let probe = gi_update_probe(id.x);
    if (probe.x == GI_NONE) {
        return;
    }
    let global = probe.x * gi.dims.w + probe.y;
    let age = min((gi_state[global] & GI_AGE) + 1u, GI_AGE);
    let moved = gi_relocate(id.x, gi_offsets[global].xyz, gi.cascades[probe.x].spacing.x);
    // (w: the openness, blended as the sky's share, `gi_blend_sky`)
    let open = gi_blend_sky(gi_offsets[global].w, gi_openness(id.x), gi_state[global]);
    gi_offsets[global] = vec4<f32>(moved.xyz, open);
    gi_state[global] = GI_VALID | select(0u, GI_INSIDE, moved.w > 0.5) | age;
}
