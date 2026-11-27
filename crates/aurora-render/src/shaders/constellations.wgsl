// The constellations and the planets of the real sky, drawn by env.wgsl's
// sky_stars. Concatenated after env.wgsl (lib.rs `ENV_SHADER`).
//
// ---------------------------------------------------------- constellations
//
// The constellations over the real sky (aurora-render constellations.rs):
// engraved figures that draw themselves like a pen stroke (a figure may
// have a drawing from each art set: its outgoing one fades while the
// incoming one draws in), their stick lines from star to star and their
// names, in the catalogue's frame, from the lists of the stars' cube cells. Only the main view's own sky draws
// them (`constellation_sky`, set by sky_pixel): not the water's bent rays,
// the planar reflections, the mirror or the probes. Off, the first test
// returns: nothing is read but the uniform.

const CONSTELLATION_FIGURES: u32 = 1536u;              // constellations.rs MAX_FIGURES
const CONSTELLATION_ATLAS: vec2<f32> = vec2<f32>(4096.0, 2048.0); // ATLAS_W, ATLAS_H
const CONSTELLATION_BLOCK: f32 = 64.0;                 // BLOCK
const CONSTELLATION_LAST_MIP: f32 = 2.0;               // ATLAS_MIPS - 1
// a cell lists a few figures, lines and names; a guard against a bad table
// (constellations.rs MAX_REFS: above the longest list of every culture
// drawn together)
const CONSTELLATION_MAX_REFS: u32 = 1024u;
// colours (preview.py of the art): the ink, the lines, the names
const CONSTELLATION_INK: vec3<f32> = vec3<f32>(0.62, 0.74, 1.0);
const CONSTELLATION_LINE: vec3<f32> = vec3<f32>(0.55, 0.70, 1.0);
const CONSTELLATION_NAME: vec3<f32> = vec3<f32>(0.40, 0.51, 0.83);
// their light against the stars' (stars.rs intensity, times the default
// star brightness: the art's preview draws stars at 0.6 of that, ink at
// 0.38 and lines at 0.3)
const CONSTELLATION_INK_GAIN: f32 = 1.25;
const CONSTELLATION_LINE_GAIN: f32 = 1.0;
const CONSTELLATION_NAME_GAIN: f32 = 1.1;
// the gap a line leaves at each of its stars (radians, 0.45°), at most
// this share of its own length (constellations.rs line_gap)
const CONSTELLATION_GAP: f32 = 0.00785;
const CONSTELLATION_GAP_SHARE: f32 = 0.15;
// head.y's bits (constellations.rs HEAD_*): drawings, a colour per figure,
// the sparkles
const CONSTELLATION_DRAWINGS: u32 = 1u;
const CONSTELLATION_TINTS: u32 = 2u;
const CONSTELLATION_SPARKLES: u32 = 4u;
// a sparkle fades out over this much of its figure's line time once its
// line is drawn: within the 0.12 a lines-only figure's stroke runs past 1
// (constellations_state.rs REVEAL_END), so the last one never stays
const CONSTELLATION_SPARK_FADE: f32 = 0.1;
// it eases in over this first share of its line (no pop)
const CONSTELLATION_SPARK_RISE: f32 = 0.03;
// its light: the line's colour towards white, brighter than the line
const CONSTELLATION_SPARK_WHITE: f32 = 0.5;
const CONSTELLATION_SPARK_GAIN: f32 = 1.6;

struct ConstellationFrame {
    // x: 1 = something shows, y: CONSTELLATION_* bits, z: vec4 index of the
    // figures' label points (1 each), w: of the drawings' states (1 each)
    head: vec4<u32>,
    // vec4 index of the references, the drawings (5 each), the segments (2
    // each), the cell table (0, or the sparkles' wider one)
    bases: vec4<u32>,
    look: vec4<f32>,   // x: the lines' half-width, y: a name texel (radians)
    // the sparkles: x radius (radians), y turns over a line, z shape, w light
    style: vec4<f32>,
    // per figure: x its name's texel (radians), y opacity, z the lines'
    // time reached, w its colour (bits of unpack4x8unorm, read with
    // bitcast; 0: the overlay's own)
    figures: array<vec4<f32>, 1536>,
    // x: the name's place (bits), y: width | height << 16 (bits), z: its
    // centre (bits: 0 the figure's label point, d + 1 drawing d's own;
    // with the top bit, the figure's, the name moved off its stars: x
    // right in bits 16..29 (bit 30 clear: never a NaN), y up in bits
    // 0..15, signed, in quarters of its texel), w: the name's opacity
    names: array<vec4<f32>, 1536>,
    // per planet, two: its direction and its name's opacity; its name's
    // place (bits), width | height << 16 (bits), texel, and how far under
    // the planet its centre goes (radians)
    planets: array<vec4<f32>, 12>,
};

@group(0) @binding(33) var<uniform> constellation_frame: ConstellationFrame;
@group(0) @binding(34) var<storage, read> constellation_data: array<vec4<u32>>;
@group(0) @binding(35) var constellation_atlas: texture_2d_array<f32>;
@group(0) @binding(36) var constellation_sampler: sampler;
// set by sky_pixel: this pixel is the view's own sky
var<private> constellation_sky: bool;

fn constellation_word(i: u32) -> u32 {
    return constellation_data[i >> 2u][i & 3u];
}

fn constellation_vec(i: u32) -> vec4<f32> {
    return bitcast<vec4<f32>>(constellation_data[i]);
}

// An atlas place (pack_place, not 0): its first texel and its layer.
fn constellation_place(bits: u32) -> vec3<f32> {
    return vec3<f32>(f32((bits >> 4u) & 255u) * CONSTELLATION_BLOCK, f32((bits >> 12u) & 255u) * CONSTELLATION_BLOCK, f32((bits & 15u) - 1u));
}

// A figure's own colour (constellations.rs pack_tint), for an element whose
// own colour is `own`: the figure's hue at that element's light.
fn constellation_tinted(bits: f32, own: vec3<f32>) -> vec3<f32> {
    return unpack4x8unorm(bitcast<u32>(bits)).rgb * max(own.r, max(own.g, own.b));
}

// A star of `points` points at `p` (pixels from its centre, turned): its
// tips at `r` (the first straight up), its inner corners at `inner` times
// `r`, a pixel of soft edge (the folded point against the edge from a tip
// to the next inner corner).
fn constellation_star(p: vec2<f32>, r: f32, points: f32, inner: f32) -> f32 {
    let seg = 6.2831853 / points;
    var a = atan2(p.x, p.y);
    a = abs(a - seg * round(a / seg));
    let q = length(p) * vec2<f32>(sin(a), cos(a));
    let tip = vec2<f32>(0.0, r);
    let corner = inner * r * vec2<f32>(sin(seg * 0.5), cos(seg * 0.5));
    let e = corner - tip;
    let out = normalize(vec2<f32>(-e.y, e.x));
    return clamp(0.5 - dot(q - tip, out), 0.0, 1.0);
}

// A sparkle's light at `p` (pixels from its centre, turned), `r` its
// radius in pixels: 0 a circle with a cross star, 1 the cross star, 2 a
// five-pointed star. Each fills its radius with full light, so that one
// size reads alike in the three shapes (a cross bright in a core a fifth
// of its radius was lost on a wide line's brighter tip).
fn constellation_spark_shape(p: vec2<f32>, r: f32, shape: u32) -> f32 {
    if (shape == 2u) {
        return constellation_star(p, r, 5.0, 0.45);
    }
    if (shape == 1u) {
        // four points along both axes, the rays reaching the radius
        return constellation_star(p, r, 4.0, 0.38);
    }
    // a soft round glow, at half its light a third of the radius out,
    // and cross rays reaching the radius past it
    let glow = exp(-dot(p, p) / (0.14 * r * r));
    return max(constellation_star(p, r, 4.0, 0.26), glow);
}

// The gap a line of `arc` radians leaves at each of its stars: 0.45°, or
// less on a short line, so that its two gaps never take more than 30 % of
// it (a short line stays a line, not a dash); the same as
// constellations.rs line_gap.
fn constellation_gap(arc: f32) -> f32 {
    return min(CONSTELLATION_GAP, CONSTELLATION_GAP_SHARE * max(arc, 0.0));
}

// The sparkle riding the pen tip of the line from `a` to `b` while it
// draws (constellation_line's front, `times` and `reached` its own), then
// fading out once it is drawn; `fig` picks the demo's mixed shapes.
fn constellation_spark(q: vec3<f32>, a: vec3<f32>, b: vec3<f32>, times: vec2<f32>, reached: f32, pixel: f32, fig: u32) -> f32 {
    let st = constellation_frame.style;
    let f = (reached - times.x) / max(times.y - times.x, 1e-4);
    let fade = 1.0 - smoothstep(0.0, CONSTELLATION_SPARK_FADE, reached - times.y);
    if (f <= 0.0 || fade <= 0.0) {
        return 0.0;
    }
    let normal = cross(a, b);
    let len = length(normal);
    let arc = atan2(len, dot(a, b));
    let gap = constellation_gap(arc);
    if (len < 1e-6 || arc <= 2.0 * gap) {
        return 0.0;
    }
    let n = normal / len;
    let side = cross(n, a);
    let g = clamp(f, 0.0, 1.0);
    let end = gap + (arc - 2.0 * gap) * g;
    let tip = a * cos(end) + side * sin(end);
    let d = q - tip;
    let r = st.x / pixel;
    let p = vec2<f32>(dot(d, side * cos(end) - a * sin(end)), dot(d, n)) / pixel;
    if (any(abs(p) > vec2<f32>(r + 1.0))) {
        return 0.0;
    }
    // its turns: st.y full turns from the line's start to its end
    let turn = 6.2831853 * st.y * g;
    let c = cos(turn);
    let s = sin(turn);
    let turned = vec2<f32>(c * p.x - s * p.y, s * p.x + c * p.y);
    let shape = select(u32(st.z), fig % 3u, st.z > 2.5);
    return constellation_spark_shape(turned, r, shape) * fade * smoothstep(0.0, CONSTELLATION_SPARK_RISE, f) * st.w;
}

// A stick line from star `a` to star `b` (unit, catalogue frame) at `q`,
// drawn from `a` as far as the figure's line time `reached` has come
// between its own `times`, leaving a gap at each star (preview.py).
// Coverage of the pixel (`pixel`: its angle).
fn constellation_line(q: vec3<f32>, a: vec3<f32>, b: vec3<f32>, times: vec2<f32>, reached: f32, pixel: f32) -> f32 {
    let f = clamp((reached - times.x) / max(times.y - times.x, 1e-4), 0.0, 1.0);
    let normal = cross(a, b);
    let len = length(normal);
    if (f <= 0.0 || len < 1e-6) {
        return 0.0;
    }
    let n = normal / len;
    let w = constellation_frame.look.x;
    let dist = abs(dot(q, n));
    if (dist > w + pixel) {
        return 0.0;
    }
    let arc = atan2(len, dot(a, b));
    let gap = constellation_gap(arc);
    if (arc <= 2.0 * gap) {
        return 0.0;
    }
    let along = atan2(dot(q, cross(n, a)), dot(q, a));
    let end = gap + (arc - 2.0 * gap) * f;
    // the pixel's width against the line's, then its two ends
    let across = max(min(dist + 0.5 * pixel, w) - max(dist - 0.5 * pixel, -w), 0.0) / pixel;
    let ends = clamp((along - gap) / pixel + 0.5, 0.0, 1.0) * clamp((end - along) / pixel + 0.5, 0.0, 1.0);
    return across * ends;
}

// The constellations at catalogue direction `q` (unit), linear, before
// the stars' fade towards the horizon and in twilight (sky_stars).
// `pixel`: the pixel's angle.
fn constellation_overlay(q: vec3<f32>, pixel: f32) -> vec3<f32> {
    // head.x: 1 the figures show, 2 the planets' names (their own pass)
    if ((constellation_frame.head.x & 1u) == 0u || !constellation_sky || in_reflection_pass()) {
        return vec3<f32>(0.0);
    }
    let cell = constellation_frame.bases.w * 4u + star_cell(q);
    let first = constellation_word(cell);
    let last = min(constellation_word(cell + 1u), first + CONSTELLATION_MAX_REFS);
    let refs = constellation_frame.bases.x * 4u;
    let bits = constellation_frame.head.y;
    let tinted = (bits & CONSTELLATION_TINTS) != 0u;
    let sparkles = (bits & CONSTELLATION_SPARKLES) != 0u;
    var ink = 0.0;
    var line = 0.0;
    var name = vec2<f32>(0.0);
    // with a colour per figure (and the sparkles): the same light, each
    // in its figure's colour; without, the sums above stay as they were
    var colored = vec3<f32>(0.0);
    for (var r = first; r < last; r++) {
        let item = constellation_word(refs + r);
        let kind = item >> 30u;
        let index = item & 0x3fffffffu;
        if (kind == 1u) {
            let rec = constellation_frame.bases.z + index * 2u;
            let ra = constellation_data[rec];
            let rb = constellation_data[rec + 1u];
            let fig = min(ra.w, CONSTELLATION_FIGURES - 1u);
            let st = constellation_frame.figures[fig];
            if (st.y > 0.0) {
                let a = bitcast<vec3<f32>>(ra.xyz);
                let b = bitcast<vec3<f32>>(rb.xyz);
                let times = unpack2x16unorm(rb.w);
                let cover = constellation_line(q, a, b, times, st.z, pixel) * st.y;
                let own = select(CONSTELLATION_LINE, constellation_tinted(st.w, CONSTELLATION_LINE), tinted && bitcast<u32>(st.w) != 0u);
                if (tinted) {
                    colored += own * (cover * CONSTELLATION_LINE_GAIN);
                } else {
                    line += cover;
                }
                if (sparkles) {
                    let spark = constellation_spark(q, a, b, times, st.z, pixel, fig) * st.y;
                    colored += mix(own, vec3<f32>(1.0), CONSTELLATION_SPARK_WHITE) * (spark * CONSTELLATION_SPARK_GAIN);
                }
            }
            continue;
        }
        if (kind == 0u) {
            // a drawing: its state (x the pen's time reached, y opacity, w
            // its place), then its texel here in its stereographic plane
            let rec = constellation_frame.bases.y + index * 5u;
            let st = constellation_vec(constellation_frame.head.w + index);
            let place = bitcast<u32>(st.w);
            if (st.y <= 0.0 || place == 0u || (constellation_frame.head.y & CONSTELLATION_DRAWINGS) == 0u) {
                continue;
            }
            let c = constellation_vec(rec);
            let cq = dot(q, c.xyz);
            if (cq <= 0.0) {
                continue;
            }
            let k = 2.0 / (1.0 + cq);
            let e = constellation_vec(rec + 1u);
            let n = constellation_vec(rec + 2u);
            let sized = constellation_vec(rec + 3u);
            let size = sized.xy;
            let at = (vec2<f32>(e.w, n.w) - vec2<f32>(dot(q, e.xyz), dot(q, n.xyz)) * k) / c.w;
            if (any(at < vec2<f32>(0.0)) || any(at >= size)) {
                continue;
            }
            // texels per pixel: the mip, and the art's brighter ink when
            // coarse (preview.py's area filter and `amp`)
            let ratio = pixel * k / c.w;
            let lod = clamp(log2(max(ratio, 1.0)), 0.0, CONSTELLATION_LAST_MIP);
            let p = constellation_place(place);
            let s = textureSampleLevel(constellation_atlas, constellation_sampler, (p.xy + at) / CONSTELLATION_ATLAS, i32(p.z), lod);
            // the pen stroke: shown behind its front, a brighter tip at it
            let ahead = st.x - s.g;
            let shown = clamp(ahead / 0.025, 0.0, 1.0);
            let tip = select(0.0, exp(-(ahead / 0.02) * (ahead / 0.02)), ahead >= 0.0);
            let amp = 0.38 * min(1.6, max(1.0, sqrt(ratio)));
            let lit = s.r * (shown * amp + tip * 0.6) * st.y;
            if (tinted) {
                let tint = constellation_frame.figures[min(bitcast<u32>(sized.z), CONSTELLATION_FIGURES - 1u)].w;
                let own = select(CONSTELLATION_INK, constellation_tinted(tint, CONSTELLATION_INK), bitcast<u32>(tint) != 0u);
                colored += own * (lit * CONSTELLATION_INK_GAIN);
            } else {
                ink += lit;
            }
        } else {
            let fig = min(index, CONSTELLATION_FIGURES - 1u);
            // the name, upright: level with the horizon at its label point,
            // read from left to right; near the zenith, where the horizon
            // gives no direction, along the camera's right
            let nm = constellation_frame.names[fig];
            let place = bitcast<u32>(nm.x);
            if (nm.w <= 0.0 || place == 0u) {
                continue;
            }
            // z: the label point (bits: 0 the figure's, d + 1 drawing d's);
            // with the top bit, the figure's, its name's centre moved off its
            // stars (constellations.rs `name_spot_word`), in texels: x right
            // from bits 16..29, y up from bits 0..15, signed quarters
            let word = bitcast<u32>(nm.z);
            let moved = (word >> 31u) != 0u;
            let own = select(word, 0u, moved);
            let spot = select(vec2<f32>(0.0), vec2<f32>(f32(bitcast<i32>(word << 2u) >> 18u), f32(bitcast<i32>(word << 16u) >> 16u)) * 0.25, moved);
            let lp = select(
                constellation_vec(constellation_frame.head.z + fig).xyz,
                constellation_vec(constellation_frame.bases.y + (max(own, 1u) - 1u) * 5u + 4u).xyz,
                own > 0u
            );
            if (dot(q, lp) < 0.9) {
                continue;
            }
            let zenith = vec3<f32>(frame.sky_stars[0].z, frame.sky_stars[1].z, frame.sky_stars[2].z);
            let level = cross(lp, zenith);
            let cam = vec3<f32>(dot(frame.sky_stars[0].xyz, frame.cam_right.xyz), dot(frame.sky_stars[1].xyz, frame.cam_right.xyz), dot(frame.sky_stars[2].xyz, frame.cam_right.xyz));
            let flat = cam - lp * dot(cam, lp);
            let along = normalize(level) * smoothstep(0.035, 0.14, length(level));
            var right = flat * (1.0 - smoothstep(0.035, 0.14, length(level))) / max(length(flat), 1e-6) + along;
            if (dot(right, right) < 0.01) {
                right = flat;
            }
            right = normalize(right);
            let up = cross(right, lp);
            let dims = bitcast<u32>(nm.y);
            // width, height and lift (NameImage::dims): the label point is
            // the name's text's centre, lift / 2 rows under the picture's
            // when a reading sits over the text
            let wh = vec2<f32>(f32(dims & 0xfffu), f32((dims >> 12u) & 0x3ffu));
            let lift = f32(dims >> 22u);
            // each name its own texel: a long one is smaller, within its reach
            let texel = constellation_frame.figures[fig].x;
            let v = q - lp * dot(q, lp);
            let at = vec2<f32>(dot(v, right), -dot(v, up)) / texel + vec2<f32>(-spot.x, spot.y) + vec2<f32>(wh.x, wh.y + lift) * 0.5;
            // no nearer the edge than half the last level's texel: the taps
            // stay within the picture, zero at its edges, never on what lies
            // around it in the atlas (constellations_names.rs `pad`)
            let edge = exp2(CONSTELLATION_LAST_MIP - 1.0);
            if (any(at < vec2<f32>(edge)) || any(at >= wh - edge)) {
                continue;
            }
            let lod = clamp(log2(max(pixel / texel, 1.0)), 0.0, CONSTELLATION_LAST_MIP);
            let p = constellation_place(place);
            let text = textureSampleLevel(constellation_atlas, constellation_sampler, (p.xy + at) / CONSTELLATION_ATLAS, i32(p.z), lod).rg * nm.w;
            let tint = constellation_frame.figures[fig].w;
            if (tinted) {
                let own = select(CONSTELLATION_NAME, constellation_tinted(tint, CONSTELLATION_NAME), bitcast<u32>(tint) != 0u);
                colored += own * ((text.x + 0.3 * text.y) * CONSTELLATION_NAME_GAIN);
            } else {
                name += text;
            }
        }
    }
    return CONSTELLATION_INK * (ink * CONSTELLATION_INK_GAIN) + CONSTELLATION_LINE * (line * CONSTELLATION_LINE_GAIN)
        + CONSTELLATION_NAME * ((name.x + 0.3 * name.y) * CONSTELLATION_NAME_GAIN) + colored;
}

// The planets at catalogue direction `q` (world/planets.rs, stars.rs
// StarFrame::planets), for sky_stars' sum: points drawn as the stars' are,
// but steady (no twinkle); a count of 0 reads nothing more. `ax`, `ay`,
// `ax2`, `ay2` and `pixel`: the pixel's footprint, as sky_stars has it.
fn sky_planet_points(q: vec3<f32>, ax: vec3<f32>, ay: vec3<f32>, ax2: f32, ay2: f32, pixel: f32) -> vec3<f32> {
    var sum = vec3<f32>(0.0);
    let planets = min(u32(frame.sky_planets[0].x), 6u);
    for (var k = 0u; k < planets; k++) {
        let pos = frame.sky_planets[1u + 2u * k];
        let tint = frame.sky_planets[2u + 2u * k];
        let off = pos.xyz - q;
        // about 1.8° away: far past its glow
        if (dot(off, off) > 1e-3) {
            continue;
        }
        let px = dot(off, ax) / ax2;
        let py = dot(off, ay) / ay2;
        let r2 = (px * px + py * py) / (tint.w * tint.w);
        let reach = STAR_MARGIN / (pixel * tint.w);
        let reach2 = reach * reach;
        if (pos.w > 0.3 && r2 < min(144.0, reach2)) {
            sum += tint.rgb * (pos.w - 0.3) * 0.05 * exp(-0.5 * r2 / 16.0);
        }
        if (r2 < min(9.0, reach2)) {
            sum += tint.rgb * pos.w * exp(-0.5 * r2);
        }
    }
    return sum;
}

// The planets' names at catalogue direction `q` (world/planets.rs): each
// centred under its planet, upright by the figures' names' rule, in the
// names' colour; nothing in reflections nor while head.x lacks its bit.
const CONSTELLATION_PLANETS: u32 = 6u;
fn constellation_planet_names(q: vec3<f32>, pixel: f32) -> vec3<f32> {
    if ((constellation_frame.head.x & 2u) == 0u || !constellation_sky || in_reflection_pass()) {
        return vec3<f32>(0.0);
    }
    var name = vec2<f32>(0.0);
    for (var k = 0u; k < CONSTELLATION_PLANETS; k++) {
        let at = constellation_frame.planets[2u * k];
        let nm = constellation_frame.planets[2u * k + 1u];
        let place = bitcast<u32>(nm.x);
        // about 14° around the planet: past its name, stacked ones too
        if (at.w <= 0.0 || place == 0u || dot(q, at.xyz) < 0.97) {
            continue;
        }
        let lp = at.xyz;
        let zenith = vec3<f32>(frame.sky_stars[0].z, frame.sky_stars[1].z, frame.sky_stars[2].z);
        let level = cross(lp, zenith);
        let cam = vec3<f32>(dot(frame.sky_stars[0].xyz, frame.cam_right.xyz), dot(frame.sky_stars[1].xyz, frame.cam_right.xyz), dot(frame.sky_stars[2].xyz, frame.cam_right.xyz));
        let flat = cam - lp * dot(cam, lp);
        let along = normalize(level) * smoothstep(0.035, 0.14, length(level));
        var right = flat * (1.0 - smoothstep(0.035, 0.14, length(level))) / max(length(flat), 1e-6) + along;
        if (dot(right, right) < 0.01) {
            right = flat;
        }
        right = normalize(right);
        let up = cross(right, lp);
        let dims = bitcast<u32>(nm.y);
        let wh = vec2<f32>(f32(dims & 0xffffu), f32(dims >> 16u));
        let texel = max(nm.z, 1e-6);
        // w: how far under the planet its name's centre goes, its text a
        // gap under it and under the names of planets in conjunction before
        // it (constellations_planets.rs `planet_names_fill`)
        let below = nm.w;
        let v = q - lp * dot(q, lp);
        let uv = vec2<f32>(dot(v, right), -dot(v, up) - below) / texel + wh * 0.5;
        // off the picture's edge, as the figures' names
        let edge = exp2(CONSTELLATION_LAST_MIP - 1.0);
        if (any(uv < vec2<f32>(edge)) || any(uv >= wh - edge)) {
            continue;
        }
        let lod = clamp(log2(max(pixel / texel, 1.0)), 0.0, CONSTELLATION_LAST_MIP);
        let p = constellation_place(place);
        name += textureSampleLevel(constellation_atlas, constellation_sampler, (p.xy + uv) / CONSTELLATION_ATLAS, i32(p.z), lod).rg * at.w;
    }
    return CONSTELLATION_NAME * ((name.x + 0.3 * name.y) * CONSTELLATION_NAME_GAIN);
}
