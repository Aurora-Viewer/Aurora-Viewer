// The shading's read of the traced global illumination (gi.rs), after
// gi_common.wgsl. The shader that includes it declares `gi` (uniform),
// `gi_state` (storage, read), `gi_irr_tex`, `gi_depth_tex` (the atlases),
// `gi_sampler` (linear, clamped), `gi_offsets` (storage, read: each
// probe's relocation), `gi_probe_seen(probe, point) -> bool`: whether
// nothing stops the straight line between a probe and the point it lights
// (gi_hit.wgsl: a ray in the scene), and
// `gi_note(kind)` and `gi_note_add(kind, n)`, which count the read's work
// for `AURORA_RT_GI_DIAG=count` (`GI_NOTE_*`; gi.rs `PLAIN_HOOKS`, nothing,
// and shaders/gi_count.wgsl).
//
// A point takes the eight probes of its cell, weighted by the trilinear
// weights, by how much each faces the normal, and by a Chebyshev test on the
// distances the probe saw in its direction (a probe behind a wall saw the
// wall nearer than the point, and counts for almost nothing): the paper's
// section 5 (Majercik et al. 2019), written from it. Near the edge of a
// cascade the next, coarser one takes over.

// How far into its cascade `pos` lies, in cells, from 0 at the last cell
// whose eight probes are all in the grid to 1 two cells further in.
fn gi_cascade_weight(c: u32, pos: vec3<f32>) -> f32 {
    let k = gi.cascades[c];
    let g = pos * k.spacing.y - vec3<f32>(k.origin.xyz);
    let hi = vec3<f32>(gi.dims.xyz) - vec3<f32>(1.0);
    let e = min(g, hi - g);
    return clamp(min(e.x, min(e.y, e.z)) * 0.5, 0.0, 1.0);
}

fn gi_atlas_uv(c: u32, s: vec3<u32>, dir: vec3<f32>, tile: u32, interior: u32, inv_size: vec2<f32>) -> vec2<f32> {
    let corner = vec2<f32>(gi_tile_origin(c, s, tile)) + vec2<f32>(1.0);
    let local = (gi_oct_encode(dir) * 0.5 + vec2<f32>(0.5)) * f32(interior);
    return (corner + local) * inv_size;
}

// Under this weight a probe's weight is crushed (gi_sample_cascade).
const GI_CRUSH: f32 = 0.2;
// The spread of a probe's distances in a depth texel (their deviation over
// their mean) under which the texel holds one surface, not a wall's edge
// blurred with what lies past it (`gi_depth_sure`).
const GI_VIS_SPREAD: f32 = 0.1;

// Whether the probe's own depth settles that it sees a point `r` from it,
// with no ray: the texel towards the point holds one surface (its spread
// is small) that is not nearer than the point. The visibility ray is for
// the texels the moments blur at a wall's edge, where the wall and what
// lies past its end average to a distance beyond the point: their spread
// is large (a wall at 2 m and a floor at 3 m behind a point 2.2 m away
// average past it only with a fifth of the floor, a spread of 18 %). In a
// busy place the rays of every probe read were most of the indirect
// light's time (a grid crowd, 2026-11-23: 3.7 ms of 7). `m`: the texel's
// mean distance and mean square.
fn gi_depth_sure(m: vec2<f32>, r: f32) -> bool {
    if (gi.frame.z == GI_VIS_ALL || r > m.x) {
        return false;
    }
    let variance = abs(m.y - m.x * m.x);
    return variance <= GI_VIS_SPREAD * GI_VIS_SPREAD * m.x * m.x;
}

// `gi.frame.z`, which probes trace their visibility ray (rt_gi_probes.rs,
// `AURORA_RT_GI_VIS`): those `gi_depth_sure` does not settle, but the ones
// `gi_tiny` drops; every one; those `gi_depth_sure` does not settle.
const GI_VIS_TINY: u32 = 0u;
const GI_VIS_ALL: u32 = 1u;
const GI_VIS_SURE: u32 = 2u;

// `gi.frame.w`'s bit that leaves out the first cascade's reads, a stress
// test of the coarser cascades' long rays (`AURORA_RT_GI_DIAG=coarse`).
const GI_COARSE: u32 = 1u;

// The weight under which a probe the depth does not settle is taken for
// hidden with no ray (`gi_tiny`): of the eight probes' trilinear weights
// (their sum is 1), the facing's (0.2 to 1) taken into it.
const GI_TINY: f32 = 0.01;

// Whether a probe weighs too little in the read to be worth its ray: its
// trilinear weight times its facing, `facing_near`, under `GI_TINY`. It is
// taken for hidden, the side that never leaks: its light, at most a
// hundredth of the weights, is left out and the others renormalised, and
// it counts that little towards `shut`. A point near a corner of its cell
// has several such probes, the same for its neighbours, so that a
// wavefront skips the ray together (a grid crowd, 2026-11-23: P1 alone
// saved 0.5 of the rays' 3.9 ms, its texels mixing several surfaces).
fn gi_tiny(facing_near: f32) -> bool {
    return gi.frame.z == GI_VIS_TINY && facing_near < GI_TINY;
}

// `gi_note`'s kinds, the work of a read (rt_gi_counts.rs `KINDS` names
// them): a cascade read (+ its index), a probe whose visibility is asked
// (its weight past the distances' test), one the depth settled, one
// `gi_tiny` dropped, a ray traced (+ the cascade's index), and a ray that
// found the probe hidden; shaders/gi_count.wgsl adds the ray's candidates
// (10 and 11). Then the length of the rays traced, in centimetres (+ the
// cascade's index, `gi_note_add`).
const GI_NOTE_READ: u32 = 0u;
const GI_NOTE_ASKED: u32 = 3u;
const GI_NOTE_SURE: u32 = 4u;
const GI_NOTE_TINY: u32 = 5u;
const GI_NOTE_RAY: u32 = 6u;
const GI_NOTE_HIDDEN: u32 = 9u;
const GI_NOTE_LENGTH: u32 = 12u;
// The sum of the eight probes' weights from which a cascade is trusted in
// full: under it, most of the weight is of probes the point hardly sees
// (behind a wall, under a floor), and their light, brought back to a whole
// by the division by the sum, was the light of the room beyond. The rest
// goes to the next cascade, then to the caller's ambient.
const GI_TRUSTED: f32 = 0.25;

// What the volume holds at a point: `light`, the irradiance (over pi) in
// rgb and in a the share of the sky the probes see along the normal (gi.rs
// `blend_sky`); `weight`, for a cascade the sum of its probes' weights, for
// the volume how much of the point it covers (0: none); `open`, how open
// the place is whatever the normal (gi_update.wgsl `gi_openness`).
struct GiRead {
    light: vec4<f32>,
    weight: f32,
    open: f32,
}

// A cascade's read: `GiRead`, and `shut` the weight of the probes around
// the point that a ray found hidden behind a face (`gi_probe_seen`), which
// `gi_read` takes for a point shut away from them: in the hollow under a
// ball or at a box's foot every probe is hidden, and what the seen ones
// left untrusted went to the whole sky (a bright halo in a dark room). The
// probes the distances already hid are not counted: under the ground they
// would darken the terrain in bands of the grid.
struct GiCascadeRead {
    light: vec4<f32>,
    weight: f32,
    shut: f32,
    open: f32,
}

// Cascade `c` at `pos` with normal `n`; `eye` from `pos` to the eye (or to
// the origin of the ray that met `pos`), as long as the distance to it;
// `lift` the share of the bias taken along the normal (`gi_read`).
fn gi_sample_cascade(c: u32, pos: vec3<f32>, n: vec3<f32>, eye: vec3<f32>, lift: f32) -> GiCascadeRead {
    let k = gi.cascades[c];
    let spacing = k.spacing.x;
    gi_note(GI_NOTE_READ + c);
    // off the surface, along the normal and towards the eye, so that the
    // probes in front of it are not hidden by the surface itself (the
    // paper's self-shadow bias, 2021 follow-up). Towards the eye at most
    // half the way there: a short ray into a room's corner met the next
    // wall a few centimetres from where it left, and the 0.36 m of the
    // first cascade's bias went back through the wall it left, outside,
    // where the probes beyond saw the point and lit it (bright bands along
    // every inner edge of a closed room, where the view's indirect light
    // reads its hits off the screen: on a turn of the camera, for a second).
    // The way back to the eye is free (the eye sees the point, the ray came
    // along it); the normal's is not, and a ray's hit takes none of it
    // (`lift` 0): under a ball resting on the floor, a short ray from the
    // floor met the ball's underside, whose normal points down, and the
    // bias along it went through the floor's slab, to the probes under it,
    // which see the sun (a bright crescent at the ball's foot in a closed
    // room). The screen's points keep it, less than half the way to the
    // eye: the view meets the faces at a grazing angle, where the way back
    // alone stays near the face.
    let bias = 0.3 * spacing;
    let far = length(eye);
    let v = eye / max(far, 1e-6);
    let biased = pos + n * min(lift * bias, 0.5 * far) + v * min(0.8 * bias, 0.5 * far);
    let g = biased * k.spacing.y - vec3<f32>(k.origin.xyz);
    let base = floor(g);
    let alpha = g - base;
    let d = vec3<i32>(gi.dims.xyz);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    var shut = 0.0;
    var open = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let o = vec3<u32>(i & 1u, (i >> 1u) & 1u, (i >> 2u) & 1u);
        let local = vec3<i32>(base) + vec3<i32>(o);
        if (any(local < vec3<i32>(0)) || any(local >= d)) {
            continue;
        }
        let cell = k.origin.xyz + local;
        let s = gi_wrap(cell);
        let global = c * gi.dims.w + gi_slot_index(s);
        // a probe inside a wall is read by nothing (the 2021 paper's
        // classification), until it has moved out
        if ((gi_state[global] & (GI_VALID | GI_INSIDE)) != GI_VALID) {
            continue;
        }
        let probe_pos = vec3<f32>(cell) * spacing + gi_offsets[global].xyz;
        // how much the probe faces the normal, never quite 0 so that a
        // point surrounded by probes behind it still takes light
        let to_probe = normalize(probe_pos - pos);
        var w = (dot(to_probe, n) + 1.0) * 0.5;
        w = w * w + 0.2;
        let tri = mix(vec3<f32>(1.0) - alpha, alpha, vec3<f32>(o));
        let near = tri.x * tri.y * tri.z;
        let facing = w;
        // visibility: the distances the probe saw towards the point
        let from_probe = biased - probe_pos;
        let r = length(from_probe);
        let dir = from_probe / max(r, 1e-4);
        let m = textureSampleLevel(gi_depth_tex, gi_sampler,
            gi_atlas_uv(c, s, dir, GI_DEPTH_TILE, GI_DEPTH_INTERIOR, gi.atlas.zw), 0.0).xy;
        if (r > m.x) {
            let variance = abs(m.y - m.x * m.x);
            let dr = r - m.x;
            let cheb = variance / (variance + dr * dr);
            w *= max(cheb * cheb * cheb, 0.0);
        }
        // the moments blur at a wall's edge: a texel of the probe beyond a
        // thin wall averages the wall and what lies past its end, and that
        // probe passed for one that sees the point along every inner edge
        // of a closed room (its sky and its light leaked in a band there).
        // A ray from the probe to the point settles it (`gi_probe_seen`,
        // the includer's: a ray query where there is a scene to trace),
        // where the probe's depth does not and it weighs enough
        if (w > 1e-3) {
            gi_note(GI_NOTE_ASKED);
            var hidden = false;
            if (gi_depth_sure(m, r)) {
                gi_note(GI_NOTE_SURE);
            } else if (gi_tiny(facing * near)) {
                gi_note(GI_NOTE_TINY);
                hidden = true;
            } else {
                gi_note(GI_NOTE_RAY + c);
                gi_note_add(GI_NOTE_LENGTH + c, u32(distance(probe_pos, biased) * 100.0));
                hidden = !gi_probe_seen(probe_pos, biased);
                if (hidden) {
                    gi_note(GI_NOTE_HIDDEN);
                }
            }
            if (hidden) {
                shut += facing * near;
                continue;
            }
        }
        // weights under GI_CRUSH crush towards 0 so that a probe mostly
        // hidden adds almost no light (RTXGI's threshold of 0.2: at 0.04 the
        // probes beyond a wall still lit the room's hits)
        w = select(w, w * w * w / (GI_CRUSH * GI_CRUSH), w < GI_CRUSH);
        w *= near;
        let irr = textureSampleLevel(gi_irr_tex, gi_sampler,
            gi_atlas_uv(c, s, n, GI_IRR_TILE, GI_IRR_INTERIOR, gi.atlas.xy), 0.0);
        sum += irr * w;
        total += w;
        open += gi_offsets[global].w * w;
    }
    if (total <= 1e-6) {
        return GiCascadeRead(vec4<f32>(0.0), 0.0, shut, 0.0);
    }
    return GiCascadeRead(sum / total, total, shut, open / total);
}

// The volume at `pos` (`GiRead`; weight 0 outside every cascade or before
// its probes have light), `eye` and `lift` as `gi_sample_cascade`'s.
fn gi_read(pos: vec3<f32>, n: vec3<f32>, eye: vec3<f32>, lift: f32) -> GiRead {
    if (gi.frame.y == 0u) {
        return GiRead(vec4<f32>(0.0), 0.0, 0.0);
    }
    var light = vec4<f32>(0.0);
    var open = 0.0;
    var left = 1.0;
    for (var c = 0u; c < GI_CASCADES; c++) {
        let w = gi_cascade_weight(c, pos);
        if (w <= 0.0 || (c == 0u && (gi.frame.w & GI_COARSE) != 0u)) {
            continue;
        }
        let s = gi_sample_cascade(c, pos, n, eye, lift);
        if (s.weight <= 0.0 && s.shut <= 0.0) {
            continue;
        }
        let take = left * w * clamp(s.weight / GI_TRUSTED, 0.0, 1.0);
        light += s.light * take;
        open += s.open * take;
        left -= take;
        // what the seen probes leave untrusted, the hidden ones say is shut
        // away: no light and no sky, in the place of the next cascade's
        // (whose probes, further, are hidden the more) and of the caller's
        // open ambient
        left -= left * w * clamp(s.shut / GI_TRUSTED, 0.0, 1.0);
        if (left <= 1e-3) {
            break;
        }
    }
    let covered = 1.0 - left;
    if (covered <= 0.0) {
        return GiRead(vec4<f32>(0.0), 0.0, 0.0);
    }
    return GiRead(light / covered, covered, open / covered);
}

// The volume's irradiance (over pi) at a ray's hit `pos`: rgb, and in a
// how much of it the volume covers, for the caller to mix with its own
// ambient. `eye` from the hit back to the ray's origin.
fn gi_irradiance(pos: vec3<f32>, n: vec3<f32>, eye: vec3<f32>) -> vec4<f32> {
    let r = gi_read(pos, n, eye, 0.0);
    return vec4<f32>(r.light.rgb, r.weight);
}

// The share of the sky's ambient that reaches `pos` with normal `n`: what
// the probes see of the sky, the whole sky where the volume does not cover
// the point (the ambient as without the probes). At a ray's hit; the
// screen's points take `gi_sky_view`.
fn gi_sky_share(r: GiRead) -> f32 {
    return clamp(mix(1.0, r.light.a, r.weight), 0.0, 1.0);
}

fn gi_sky(pos: vec3<f32>, n: vec3<f32>, eye: vec3<f32>) -> f32 {
    return gi_sky_share(gi_read(pos, n, eye, 0.0));
}

// How open the place around `pos` is, a point with no face of its own (a
// reflection probe's centre, a place in front of a mirror): the volume's
// openness read along the six axes and averaged by how much each read
// covers, 1 where it covers nothing. rt_gi.wgsl `gi_points`.
fn gi_open_at(pos: vec3<f32>) -> f32 {
    var open = 0.0;
    var weight = 0.0;
    for (var a = 0u; a < 6u; a++) {
        let axis = a % 3u;
        let s = select(-1.0, 1.0, a < 3u);
        let n = vec3<f32>(select(0.0, s, axis == 0u), select(0.0, s, axis == 1u), select(0.0, s, axis == 2u));
        let r = gi_read(pos, n, vec3<f32>(0.0), 0.0);
        open += r.open * r.weight;
        weight += r.weight;
    }
    if (weight <= 0.0) {
        return 1.0;
    }
    return clamp(mix(1.0, open / weight, weight / 6.0), 0.0, 1.0);
}

// At a point the camera sees, `eye` from it to the camera: `gi_sky` in x,
// and in y how open the place is (`GiRead::open`), 1 where the volume does
// not cover the point.
fn gi_sky_view(pos: vec3<f32>, n: vec3<f32>, eye: vec3<f32>) -> vec2<f32> {
    let r = gi_read(pos, n, eye, 0.2);
    return vec2<f32>(gi_sky_share(r), clamp(mix(1.0, r.open, r.weight), 0.0, 1.0));
}
