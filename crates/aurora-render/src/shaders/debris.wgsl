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
        // an ember: a point that faces the camera
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

// A cheap hash of the pixel for the hashed alpha test (stable per pixel
// and frame: TAA smooths it into coverage).
fn dd_dither(frag: vec2<f32>) -> f32 {
    let n = vec2<u32>(frag) + vec2<u32>(frame.anim_clock.x * 7u, 0u);
    return f32((n.x * 1973u + n.y * 9277u + 26699u) * 0x9e3779b9u >> 8u) / 16777216.0;
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
    // the stem: a thin line below the blade
    let stem = select(0.0, 0.06 - abs(uv.y), uv.x < -0.82);
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
    if (kind == DD_LEAVES) {
        let d = dd_leaf_shape(in.uv, variant);
        cover = clamp(d / max(fwidth(d), 1e-4) + 0.5, 0.0, 1.0);
        base = dd_leaf_colour(variant);
        // the midrib and the veins, darker; a little mottling
        let rib = 1.0 - smoothstep(0.0, 0.05, abs(in.uv.y));
        let veins = 1.0 - smoothstep(0.0, 0.06, abs(fract((in.uv.x - abs(in.uv.y) * 0.8) * 4.0) - 0.5) - 0.44);
        base *= 1.0 - 0.25 * max(rib, veins * 0.6);
        base *= 0.85 + 0.3 * fract(sin(dot(floor(in.uv * 6.0), vec2<f32>(12.9898, 78.233)) + f32(variant)) * 43758.5);
        through = 0.55;
    } else if (kind == DD_PAPER) {
        let edge = 1.0 - max(abs(in.uv.x), abs(in.uv.y) / 0.98);
        cover = clamp(edge / max(fwidth(edge), 1e-4), 0.0, 1.0);
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
        base = vec3<f32>(0.62, 0.49, 0.32) * (0.85 + 0.3 * f32(variant & 7u) / 7.0);
    } else if (kind == DD_EMBERS) {
        let r = length(in.uv);
        cover = 1.0 - smoothstep(0.3, 1.0, r);
        // it glows and flickers, cooling as it dies
        let flicker = 0.7 + 0.3 * sin(frame.camera_pos.w * (11.0 + f32(variant & 15u)) + f32(variant));
        let heat = life * flicker;
        emissive = mix(vec3<f32>(1.0, 0.12, 0.01), vec3<f32>(1.0, 0.55, 0.12), heat) * (6.0 * heat + 0.5) * (1.0 - smoothstep(0.0, 0.8, r));
        base = vec3<f32>(0.05);
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
    var color = base * (ambient * 0.9 + sun * shadow * (lit_face + behind * through * 2.5));
    color += emissive;
    // the weather's tint carries to the debris (blood rain, ash...) only
    // through the light; the fog as everything else
    return vec4<f32>(apply_fog(color, in.world), 0.0);
}
