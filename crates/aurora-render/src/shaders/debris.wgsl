// Aurora Weather's debris, drawn (debris.rs; simulated by debris_sim.wgsl):
// part of the scene module (after common.wgsl, object.wgsl and env.wgsl).
// One instance a particle, its four vec4 the instance's attributes; a free
// slot draws nothing.
//
// Leaves and paper are cards turned as the particle is (not facing the
// camera): two-sided, creased along their length (a leaf's midrib), their
// outline drawn in the fragment shader (its edge antialiased by a hashed
// test against its coverage, so no atlas and no mips to keep), lit from
// both faces, the sun shining through a leaf seen against it. Sand is a
// short streak along its velocity; an ember a glowing point.

struct DebrisOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // x along the card (-1..1), y across it
    @location(2) uv: vec2<f32>,
    // x the kind, y the variant, z its life (0..1), w how fast it moves
    @location(3) @interpolate(flat) info: vec4<f32>,
}

// debris_sim.wgsl's kinds and state bits
const DD_LEAVES: u32 = 1u;
const DD_SAND: u32 = 2u;
const DD_PAPER: u32 = 3u;
const DD_EMBERS: u32 = 4u;
const DD_MOTES: u32 = 5u;

// The user's pictures for a kind (0 leaves, 1 paper): how many, and the
// `k`th's slot.
fn dd_picture_count(which: u32) -> u32 {
    var n = 0u;
    for (var i = 0u; i < 8u; i++) {
        if (frame.debris_pictures[which * 2u + i / 4u][i % 4u] != 0u) { n = i + 1u; }
    }
    return n;
}

fn dd_picture(which: u32, k: u32) -> u32 {
    return frame.debris_pictures[which * 2u + k / 4u][k % 4u];
}

fn dd_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

fn dd_none() -> DebrisOut {
    var out: DebrisOut;
    out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    out.world = vec3<f32>(0.0);
    out.normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.info = vec4<f32>(0.0);
    return out;
}

// A card's 12 corners: two halves either side of its length (x), the
// middle line raised by the crease.
fn dd_card_corner(vertex: u32) -> vec2<f32> {
    let half = vertex / 6u;
    let k = vertex % 6u;
    // a quad from y = 0 to y = side
    let side = select(-1.0, 1.0, half == 1u);
    let quad = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = quad[k];
    return vec2<f32>(c.x, c.y * side);
}

@vertex
fn vs_debris(
    @builtin(vertex_index) vertex: u32,
    @location(0) p: vec4<f32>,
    @location(1) v: vec4<f32>,
    @location(2) q: vec4<f32>,
    @location(3) s: vec4<f32>,
) -> DebrisOut {
    if (p.w <= 0.0) { return dd_none(); }
    let state = bitcast<u32>(v.w);
    let kind = state & 15u;
    let variant = (state >> 8u) & 255u;
    let size = s.y;
    let cam = frame.camera_pos.xyz;
    var out: DebrisOut;
    out.info = vec4<f32>(f32(kind), f32(variant), p.w, length(v.xyz));
    if (kind == DD_LEAVES || kind == DD_PAPER) {
        let c = dd_card_corner(vertex);
        // a leaf is longer than wide, its outline from the variant; paper
        // a sheet, a little crumpled
        var extent = vec2<f32>(1.0, 0.55 + 0.25 * f32(variant & 3u) / 3.0);
        var crease = 0.18;
        if (kind == DD_PAPER) {
            extent = vec2<f32>(1.0, 0.75);
            crease = 0.08 + 0.1 * f32(variant & 7u) / 7.0;
        }
        // a picture of the user's: a square card, its own outline in it
        if (dd_picture_count(select(0u, 1u, kind == DD_PAPER)) > 0u) {
            extent = vec2<f32>(1.0, 1.0);
        }
        let local = vec3<f32>(c.x * extent.x, c.y * extent.y, crease * extent.y * (1.0 - abs(c.y))) * size;
        let world = p.xyz + dd_rotate(q, local);
        // the face's normal, bent by the crease on each half
        let half_n = normalize(vec3<f32>(0.0, -sign(c.y + 1e-4) * crease, 1.0));
        out.normal = dd_rotate(q, half_n);
        out.world = world;
        out.clip = world_clip(world);
        out.uv = c;
        return out;
    }
    if (vertex >= 6u) { return dd_none(); }
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vertex];
    let to_cam = normalize(cam - p.xyz);
    var along: vec3<f32>;
    var across: vec3<f32>;
    var length_m = size;
    var width_m = size;
    if (kind == DD_SAND) {
        // a grain's streak: as long as it moves in a 1/60 s, a little wider
        // than a grain so that it is seen
        let speed = length(v.xyz);
        along = select(normalize(v.xyz), vec3<f32>(0.0, 0.0, 1.0), speed < 1e-3);
        length_m = max(speed * 0.025, size);
        width_m = size * 0.6;
        across = normalize(cross(along, to_cam) + vec3<f32>(1e-5, 0.0, 0.0));
    } else {
        // an ember or a mote: a point that faces the camera
        across = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), to_cam) + vec3<f32>(1e-5, 0.0, 0.0));
        along = cross(to_cam, across);
    }
    // never thinner than about a pixel, faint by as much (as the rain)
    let dist = distance(cam, p.xyz);
    let pixel = dist * 1.15 * frame.screen.w;
    let widened = max(width_m, pixel);
    let world = p.xyz + along * c.y * length_m + across * c.x * widened;
    out.world = world;
    out.clip = world_clip(world);
    out.normal = to_cam;
    out.uv = c;
    out.info.z = p.w * min(width_m / widened, 1.0);
    return out;
}

// The hashed alpha test's threshold at a pixel: interleaved gradient
// noise, moved a little each frame (TAA smooths it into the coverage; the
// edge is the only place it decides, the rest of the card being 0 or 1).
fn dd_dither(frag: vec2<f32>) -> f32 {
    let shift = f32(frame.anim_clock.x % 64u) * 5.588238;
    return fract(52.9829189 * fract(dot(frag + vec2<f32>(shift), vec2<f32>(0.06711056, 0.00583715))));
}

// A leaf's outline at `uv` (x along it from stem -1 to tip 1, y across):
// > 0 inside. Its shape from the variant: oval, pointed, lobed (oak-like).
fn dd_leaf_shape(uv: vec2<f32>, variant: u32) -> f32 {
    let t = uv.x * 0.5 + 0.5;
    let shape = variant % 3u;
    // the half width along the leaf
    var w = pow(max(sin(3.14159 * pow(t, 0.8)), 0.0), 0.75);
    if (shape == 1u) {
        w = pow(max(sin(3.14159 * t), 0.0), 1.3) * (1.0 - 0.3 * t);
    } else if (shape == 2u) {
        // lobes
        w = pow(max(sin(3.14159 * t), 0.0), 0.6) * (0.75 + 0.25 * cos(t * 6.2831853 * 3.5));
    }
    // a fine toothed edge
    w *= 1.0 - 0.06 * abs(sin(t * 60.0));
    // the stem: a thin line below the blade (elsewhere none: well outside,
    // not on the edge, or the whole card would be half covered)
    let stem = select(-1.0, 0.06 - abs(uv.y), uv.x < -0.82);
    return max(w - abs(uv.y), stem);
}

// The leaves' colours (sRGB-ish linear): greens through autumn's.
fn dd_leaf_colour(variant: u32) -> vec3<f32> {
    let k = (variant >> 2u) % 8u;
    let palette = array<vec3<f32>, 8>(
        vec3<f32>(0.20, 0.32, 0.06), vec3<f32>(0.42, 0.40, 0.05),
        vec3<f32>(0.62, 0.36, 0.03), vec3<f32>(0.58, 0.16, 0.03),
        vec3<f32>(0.40, 0.08, 0.03), vec3<f32>(0.30, 0.17, 0.06),
        vec3<f32>(0.55, 0.45, 0.10), vec3<f32>(0.22, 0.12, 0.05));
    return palette[k];
}

@fragment
fn fs_debris(in: DebrisOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let kind = u32(in.info.x);
    let variant = u32(in.info.y);
    let life = in.info.z;
    let view = normalize(frame.camera_pos.xyz - in.world);
    let dither = dd_dither(in.clip.xy);
    var base = vec3<f32>(0.5);
    var emissive = vec3<f32>(0.0);
    var cover = 1.0;
    var through = 0.0;
    let pictures = select(0u, dd_picture_count(select(0u, 1u, kind == DD_PAPER)), kind == DD_LEAVES || kind == DD_PAPER);
    if (pictures > 0u) {
        // one of the user's pictures, the particle's own: its colour and
        // its cut-out (along the card is up the picture, its stem at the
        // bottom)
        let slot = dd_picture(select(0u, 1u, kind == DD_PAPER), variant % pictures);
        let c = sample_tex(slot, vec2<f32>(in.uv.y * 0.5 + 0.5, 0.5 - in.uv.x * 0.5));
        cover = c.a;
        base = c.rgb;
        through = select(0.3, 0.55, kind == DD_LEAVES);
    } else if (kind == DD_LEAVES) {
        let d = dd_leaf_shape(in.uv, variant);
        cover = clamp(d / max(fwidth(d), 1e-4) + 0.5, 0.0, 1.0);
        base = dd_leaf_colour(variant);
        // the midrib and the veins, darker; a little mottling
        let rib = 1.0 - smoothstep(0.0, 0.05, abs(in.uv.y));
        let veins = 1.0 - smoothstep(0.0, 0.06, abs(fract((in.uv.x - abs(in.uv.y) * 0.8) * 4.0) - 0.5) - 0.44);
        base *= 1.0 - 0.25 * max(rib, veins * 0.6);
        // soft blotches, no cells
        let v = f32(variant);
        base *= 0.9 + 0.12 * sin(in.uv.x * 5.3 + v) * sin(in.uv.y * 7.1 + v * 0.37) + 0.06 * sin((in.uv.x + in.uv.y) * 11.0 + v);
        through = 0.55;
    } else if (kind == DD_PAPER) {
        let edge = 1.0 - max(abs(in.uv.x), abs(in.uv.y) / 0.98);
        cover = clamp(edge / max(fwidth(edge), 1e-4) + 0.5, 0.0, 1.0);
        // newsprint, a flyer or a wrapper; the newsprint with its lines
        let k = variant % 4u;
        base = select(select(vec3<f32>(0.72, 0.70, 0.64), vec3<f32>(0.75, 0.75, 0.73), k == 1u), vec3<f32>(0.55, 0.2, 0.15), k == 3u);
        if (k == 0u) {
            let line = step(0.55, fract(in.uv.y * 9.0)) * step(abs(in.uv.x), 0.85);
            base *= 1.0 - 0.35 * line;
        }
        through = 0.3;
    } else if (kind == DD_SAND) {
        // a soft streak, fading at its ends
        cover = (1.0 - smoothstep(0.4, 1.0, abs(in.uv.x))) * (1.0 - smoothstep(0.5, 1.0, abs(in.uv.y))) * 0.8;
        // (a grain's own colour, darker than the sand lying in the light:
        // it is lit as it is seen, mostly from the sky and the sun's colour)
        base = SAND_COLOUR * 0.6 * (0.85 + 0.3 * f32(variant & 7u) / 7.0);
    } else if (kind == DD_EMBERS) {
        let r = length(in.uv);
        cover = 1.0 - smoothstep(0.3, 1.0, r);
        // it glows and flickers, cooling as it dies
        let flicker = 0.7 + 0.3 * sin(frame.camera_pos.w * (11.0 + f32(variant & 15u)) + f32(variant));
        let heat = life * flicker;
        emissive = mix(vec3<f32>(1.0, 0.12, 0.01), vec3<f32>(1.0, 0.55, 0.12), heat) * (6.0 * heat + 0.5) * (1.0 - smoothstep(0.0, 0.8, r));
        base = vec3<f32>(0.05);
    } else if (kind == DD_MOTES) {
        // a speck of dust: seen where light catches it, a sunbeam or a
        // lamp, the more seen toward the light (it scatters it forward),
        // barely in the shade; each its own brightness, twinkling as it
        // turns
        let r = length(in.uv);
        cover = 1.0 - smoothstep(0.2, 1.0, r);
        let toward_sun = max(dot(-view, frame.sun_dir.xyz), 0.0);
        let beam = frame.sun_color.rgb * frame.sun_dir.w * (0.1 + 3.0 * pow(toward_sun, 8.0)) * shadow_factor(in.world, view, in.clip.w);
        let lamps = rain_lamps(in.world, in.clip.xy, view);
        let sky_light = frame.sky_zenith.rgb * 0.55 + frame.sky_horizon.rgb * 0.45;
        let own = 0.35 + 1.3 * fract(f32(variant) * 0.618034);
        let twinkle = 0.55 + 0.45 * sin(frame.camera_pos.w * (0.7 + f32(variant & 7u) * 0.35) + f32(variant));
        emissive = vec3<f32>(0.85, 0.8, 0.72) * (beam + lamps * 1.5 + sky_light * 0.25) * own * twinkle;
        base = vec3<f32>(0.0);
        cover *= clamp(dot(emissive, vec3<f32>(0.2126, 0.7152, 0.0722)) * 6.0, 0.08, 1.0);
    } else {
        discard;
    }
    // what is left of it as it fades out
    cover *= clamp(life * 3.0, 0.0, 1.0);
    if (cover <= dither) { discard; }

    // two-sided: the face toward the camera
    var n = normalize(in.normal);
    if (dot(n, view) < 0.0) { n = -n; }
    let sun_dir = frame.sun_dir.xyz;
    let sun = frame.sun_color.rgb * frame.sun_dir.w;
    let shadow = shadow_factor(in.world, n, in.clip.w);
    let lit_face = max(dot(n, sun_dir), 0.0);
    // the sun through it, seen against the sun (a leaf glows)
    let behind = pow(max(dot(-view, sun_dir), 0.0), 3.0) * max(-dot(n, sun_dir) * 0.5 + 0.5, 0.0);
    let ambient = frame.sky_zenith.rgb * 0.55 + frame.sky_horizon.rgb * 0.45;
    // (a streak or a point faces the camera: half its light from the sun
    // whichever way, as a grain lit on its sunny side)
    let facing = select(lit_face, 0.5, kind == DD_SAND);
    // and the lamps around (a leaf under a street light)
    let lamps_here = select(rain_lamps(in.world, in.clip.xy, view), vec3<f32>(0.0), kind == DD_MOTES || kind == DD_EMBERS);
    var color = base * (ambient * 0.9 + sun * shadow * (facing + behind * through * 2.5) + lamps_here * 0.8);
    color += emissive;
    // the weather's tint carries to the debris (blood rain, ash...) only
    // through the light; the fog as everything else
    return vec4<f32>(apply_fog(color, in.world), 0.0);
}
