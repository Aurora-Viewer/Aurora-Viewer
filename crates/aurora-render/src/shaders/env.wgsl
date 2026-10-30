// Sky, terrain and water. Concatenated after common.wgsl and object.wgsl.
//
// Water wave layering and fresnel follow the Second Life viewer's water
// shaders (class3/environment/waterF.glsl, class1/environment/waterV.glsl),
// Copyright (C) Linden Research, Inc., LGPL 2.1.

// ------------------------------------------------------------------- sky

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var out: SkyOut;
    let p = vec2<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0);
    // reverse-Z: far plane is 0
    out.clip = vec4<f32>(p, 0.0, 1.0);
    out.ndc = p;
    return out;
}

// ------------------------------------------------------------- night sky
//
// The stars of stars.rs: a catalogue sorted into the cells of a cube map,
// each star listed in every cell its point reaches, so that a pixel reads
// its own cell only. `frame.sky_stars` turns the region's directions into
// the catalogue's frame.

const STAR_FACE_CELLS: u32 = 24u;   // stars.rs FACE_CELLS
const STAR_CELL_TABLE: u32 = 3460u; // stars.rs CELL_TABLE
const STAR_MARGIN: f32 = 0.021;     // stars.rs MARGIN (radians)
// a cell holds about 6 stars, up to a few dozen in the Milky Way
const STAR_MAX_PER_CELL: u32 = 96u;

struct GpuStar {
    pos: vec4<f32>,   // unit direction, peak intensity
    color: vec4<f32>, // linear colour, point size (pixels)
};

struct StarSky {
    cells: array<u32, 6920>, // 2 * STAR_CELL_TABLE: real sky, then the Legacy Viewer's
    stars: array<GpuStar>,
};

@group(0) @binding(24) var<storage, read> star_sky: StarSky;

// stars.rs cell_of
fn star_cell(q: vec3<f32>) -> u32 {
    let a = abs(q);
    var face: u32;
    var uv: vec2<f32>;
    var m: f32;
    if (a.x >= a.y && a.x >= a.z) {
        face = select(1u, 0u, q.x >= 0.0);
        uv = q.yz;
        m = a.x;
    } else if (a.y >= a.z) {
        face = select(3u, 2u, q.y >= 0.0);
        uv = q.xz;
        m = a.y;
    } else {
        face = select(5u, 4u, q.z >= 0.0);
        uv = q.xy;
        m = a.z;
    }
    let n = f32(STAR_FACE_CELLS);
    let c = vec2<u32>(clamp(floor((uv / max(m, 1e-20) * 0.5 + 0.5) * n), vec2<f32>(0.0), vec2<f32>(n - 1.0)));
    return face * STAR_FACE_CELLS * STAR_FACE_CELLS + c.y * STAR_FACE_CELLS + c.x;
}

// Angle (radians) one pixel of this pass covers at the middle of its view.
fn sky_pixel_angle() -> f32 {
    let a = frame.inv_view_proj_rot * vec4<f32>(0.0, 0.0, 0.5, 1.0);
    let b = frame.inv_view_proj_rot * vec4<f32>(0.0, 2.0 * frame.screen.w, 0.5, 1.0);
    return max(length(normalize(b.xyz / b.w) - normalize(a.xyz / a.w)), 1e-6);
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

// PCG hash of a lattice point (Jarzynski and Olano 2020, pcg3d): integer
// only, so it stays random far from the origin, unlike a sine hash.
fn pcg3(v0: vec3<u32>) -> vec3<u32> {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    return v;
}

fn lattice_gradient(i: vec3<f32>) -> vec3<f32> {
    let h = pcg3(vec3<u32>(vec3<i32>(i) + vec3<i32>(4096)));
    return normalize(vec3<f32>(h & vec3<u32>(0xffffu)) / 32767.5 - 1.0 + vec3<f32>(1e-4));
}

fn corner(i: vec3<f32>, f: vec3<f32>, o: vec3<f32>) -> f32 {
    return dot(lattice_gradient(i + o), f - o);
}

// Gradient noise in 0..1, quintic fade: no facets along the lattice (the
// value noise it replaces drew straight edges across the Milky Way).
fn gradient_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x00 = mix(corner(i, f, vec3<f32>(0.0, 0.0, 0.0)), corner(i, f, vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(corner(i, f, vec3<f32>(0.0, 1.0, 0.0)), corner(i, f, vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(corner(i, f, vec3<f32>(0.0, 0.0, 1.0)), corner(i, f, vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(corner(i, f, vec3<f32>(0.0, 1.0, 1.0)), corner(i, f, vec3<f32>(1.0, 1.0, 1.0)), u.x);
    let n = mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
    return clamp(0.5 + 0.75 * n, 0.0, 1.0);
}

// `octaves` octaves, each turned by a fixed rotation so that the lattices
// do not line up from one octave to the next.
fn fbm3(p: vec3<f32>, octaves: i32) -> f32 {
    let turn = mat3x3<f32>(
        vec3<f32>(0.00, 0.80, 0.60),
        vec3<f32>(-0.80, 0.36, -0.48),
        vec3<f32>(-0.60, -0.48, 0.64),
    );
    var sum = 0.0;
    var amp = 0.5;
    var total = 0.0;
    var q = turn * p;
    for (var i = 0; i < octaves; i++) {
        sum += gradient_noise(q) * amp;
        total += amp;
        q = turn * q * 2.03 + vec3<f32>(1.7, 9.2, 5.3);
        amp *= 0.5;
    }
    return sum / total;
}

// The Milky Way at catalogue direction `q` (J2000 equatorial), linear.
// Procedural, laid on the real galactic plane: a band a few degrees wide,
// brighter and wider towards the centre (Sagittarius, l = 0), faint
// towards the anticentre (Auriga), with the dark Great Rift from Cygnus to
// Sagittarius just north of the plane, and mottling.
fn milky_way(q: vec3<f32>) -> vec3<f32> {
    // J2000 equatorial to galactic (IAU 1958, Hipparcos values)
    let g = vec3<f32>(
        dot(vec3<f32>(-0.0548756, -0.8734371, -0.4838350), q),
        dot(vec3<f32>(0.4941094, -0.4448296, 0.7469822), q),
        dot(vec3<f32>(-0.8676661, -0.1980764, 0.4559838), q),
    );
    let l = atan2(g.y, g.x);
    let b = asin(clamp(g.z, -1.0, 1.0));
    let centre = exp(-l * l / (0.5 * 0.5));
    let width = 0.075 + 0.1 * centre;
    let along = 0.3 + 0.7 * (0.5 + 0.5 * cos(l));
    let band = exp(-(b * b) / (width * width)) * along;
    let bulge = exp(-(l * l + 2.4 * b * b) / (0.2 * 0.2));
    // most of the sky is far from the band: no noise there
    if (band + bulge < 0.004) {
        return vec3<f32>(0.0);
    }
    let coarse = fbm3(g * 5.0, 4);
    let fine = fbm3(g * 21.0 + vec3<f32>(7.1, 3.3, 1.9), 3);
    // star clouds: the band is patchy, not a smooth stripe
    let clouds = smoothstep(0.3, 0.75, coarse) * (0.45 + 0.75 * fine);
    var light = band * (0.15 + 1.2 * clouds) + bulge * (0.6 + 0.6 * fine);
    // the Great Rift: l from -10° to 90°, centred a little north of the plane
    let rift_l = smoothstep(-0.35, -0.1, l) * smoothstep(1.75, 1.3, l);
    let rift_b = (b - 0.025 - 0.02 * sin(l * 3.0)) / 0.035;
    let rift_shape = exp(-rift_b * rift_b) * rift_l;
    if (rift_shape > 0.01) {
        light *= 1.0 - 0.8 * rift_shape * (0.55 + 0.45 * fbm3(g * 13.0 + vec3<f32>(2.0), 3));
    }
    let tint = mix(vec3<f32>(0.80, 0.86, 1.0), vec3<f32>(1.0, 0.86, 0.68), clamp(centre * 0.7 + bulge, 0.0, 1.0));
    return tint * light * 0.09;
}

// Stars and Milky Way in region direction `d` (unit), linear, before the
// clouds' cover. `background` is the sky's own linear colour there: in the
// real sky a bright sky drowns them (twilight shows the brightest first,
// daylight none), where the Legacy Viewer adds its stars whatever the sky.
// LLDrawPoolWLSky::renderStarsDeferred passes the sky's star_brightness / 500, which starsF.glsl turns into
// smoothstep(0, 0.9, ...) * 32 on the star texture's alpha: full brightness
// from about 100, dimmer only below. The same curve, without the 32 times
// overdrive.
fn sky_stars(d: vec3<f32>, background: vec3<f32>) -> vec3<f32> {
    let catalogue = frame.sky_stars[0].w;
    let gain = min(smoothstep(0.0, 0.9, frame.sky_misc.w / 500.0) * 8.0, 1.0);
    if (catalogue < 0.0 || gain <= 0.0 || d.z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let q = vec3<f32>(dot(frame.sky_stars[0].xyz, d), dot(frame.sky_stars[1].xyz, d), dot(frame.sky_stars[2].xyz, d));
    let legacy = catalogue > 0.5;
    // atmospheric extinction: stars dim and vanish towards the horizon
    let airmass = 1.0 / (d.z + 0.025);
    let extinction = select(exp(-0.3 * (airmass - 1.0)), 1.0, legacy);
    let glow = dot(background, vec3<f32>(0.2126, 0.7152, 0.0722)) / 0.045;
    let glow2 = glow * glow;
    let glare = select(1.0 / (1.0 + glow2 * glow2), 1.0, legacy);
    // daylight, or right on the horizon: nothing would show
    if (gain * extinction * glare < 0.003) {
        return vec3<f32>(0.0);
    }
    let pixel = sky_pixel_angle();
    let t = frame.camera_pos.w;
    let cell = select(0u, STAR_CELL_TABLE, legacy) + star_cell(q);
    let first = star_sky.cells[cell];
    let last = min(star_sky.cells[cell + 1u], first + STAR_MAX_PER_CELL);
    var sum = vec3<f32>(0.0);
    for (var i = first; i < last; i++) {
        let s = star_sky.stars[i];
        let sigma = min(pixel * s.color.w, STAR_MARGIN / 12.0);
        let off = q - s.pos.xyz;
        let r2 = dot(off, off) / (sigma * sigma);
        // the bright stars' soft glow, four times wider (the eye's
        // scattering, as on a photograph of the sky)
        if (s.pos.w > 0.3 && r2 < 144.0) {
            sum += s.color.rgb * (s.pos.w - 0.3) * 0.05 * exp(-0.5 * r2 / 16.0);
        }
        if (r2 < 9.0) {
            // scintillation: slow and slight overhead, stronger low down
            // (the Legacy Viewer's stars flicker, starsF.glsl twinkle)
            let seed = hash3(floor(s.pos.xyz * 4096.0));
            let depth = select(mix(0.35, 0.05, clamp(d.z * 2.0, 0.0, 1.0)), 0.4, legacy);
            let wave = sin(t * (2.3 + 3.1 * seed) + seed * 40.0) * sin(t * (0.9 + 1.3 * seed) + seed * 17.0);
            sum += s.color.rgb * s.pos.w * exp(-0.5 * r2) * (1.0 + depth * wave);
        }
    }
    var col = sum;
    if (frame.sky_stars[1].w > 0.0) {
        col += milky_way(q) * frame.sky_stars[1].w;
    }
    return col * gain * extinction * glare;
}

// Textured (or analytic) disc of a heavenly body; rgb premultiplied, a coverage.
fn body_disc(dir: vec3<f32>, body: vec3<f32>, half_size: f32, slot: u32) -> vec4<f32> {
    let b = normalize(body);
    let mu = dot(dir, b);
    if (mu < 0.0) {
        return vec4<f32>(0.0);
    }
    var right = cross(b, vec3<f32>(0.0, 0.0, 1.0));
    if (dot(right, right) < 1e-6) {
        right = vec3<f32>(1.0, 0.0, 0.0);
    }
    right = normalize(right);
    let up = cross(right, b);
    let p = dir / mu - b;
    let uv = vec2<f32>(dot(p, right), -dot(p, up)) / half_size * 0.5 + vec2<f32>(0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return vec4<f32>(0.0);
    }
    let t = sample_tex_level(slot, uv, 0.0);
    return vec4<f32>(srgb_to_linear(t.rgb) * t.a, t.a);
}

// Rainbow and 22° halo, port of skyF.glsl's rainbow() and halo22() (Second
// Life viewer, Copyright (C) Linden Research, Inc., LGPL 2.1). `d` is the
// cosine between the view and the light. The textures hold Second Life's
// coordinates, whose v runs up the image: sampled at 1 - v, with the repeat
// wrap the GL texture had.
fn sky_rainbow(d: f32) -> vec3<f32> {
    let moisture = frame.sky_optics.x;
    let slot = frame.sky_optics_tex.x;
    if (slot == 0u || moisture <= 0.0) {
        return vec3<f32>(0.0);
    }
    // the colors sit at v 0.175..0.25 (view nearly opposite the light);
    // above 0.25 the interior is stretched over the rest of the texture
    var v = clamp(-0.575 - d, 0.0, 1.0);
    v = clamp(v, 0.0, 0.25) + max(0.0, v - 0.25) * 4.2857;
    let rad = (frame.sky_optics.y - 5.0) / 1024.0;
    let t = sample_tex_level(slot, vec2<f32>(rad + 0.5, 1.0 - v), 0.0).rgb;
    return pow(t, vec3<f32>(1.8)) * moisture;
}

fn sky_halo(d: f32) -> vec3<f32> {
    let ice = frame.sky_optics.z;
    let slot = frame.sky_optics_tex.y;
    if (slot == 0u || ice <= 0.0) {
        return vec3<f32>(0.0);
    }
    let c = clamp(d, 0.1, 1.0);
    let v = sqrt(clamp(1.0 - c * c, 0.0, 1.0));
    return sample_tex_level(slot, vec2<f32>(0.0, 1.0 - v), 0.0).rgb * ice;
}

fn sky_full(dir: vec3<f32>) -> vec3<f32> {
    let d = normalize(dir);
    let a = atmos_eval(d, false);
    // skyF.glsl: added to the haze color before its x 2, under the clouds
    let haze = a.color + sky_rainbow(a.light_dot) + sky_halo(a.light_dot);
    var c = clamp(haze * 2.0, vec3<f32>(0.0), vec3<f32>(5.0));
    let clouds = sky_clouds(d);
    c = mix(c, clouds.rgb, clouds.a);
    var col = srgb_to_linear(c) * frame.sky_cloud_pd2.w;
    let cover = 1.0 - clouds.a;
    // stars and Milky Way (above), behind the clouds
    col += sky_stars(d, col) * cover;
    // moon
    if (frame.tex_slots.z != 0u) {
        let m = body_disc(d, frame.sky_moon.xyz, 0.035 * frame.sky_moon.w, frame.tex_slots.z);
        col = col * (1.0 - m.a * 0.9 * cover) + m.rgb * frame.sky_misc.z * 2.0 * cover;
    }
    // sun
    let sd = normalize(frame.sky_sun.xyz);
    if (frame.tex_slots.y != 0u) {
        let s = body_disc(d, sd, 0.025 * frame.sky_sun.w, frame.tex_slots.y);
        col += s.rgb * 6.0 * cover;
    } else {
        let mu = dot(d, sd);
        let disk = smoothstep(0.99955, 0.9997, mu) * clamp(sd.z * 10.0 + 1.0, 0.0, 1.0);
        col += frame.sun_color.rgb * disk * 6.0 * cover;
    }
    return col;
}

fn sky_pixel(in: SkyOut) -> vec4<f32> {
    // from the camera, not region positions minus the camera: those rounded
    // differently each frame and the sky shook as it moved (#1059)
    let w = frame.inv_view_proj_rot * vec4<f32>(in.ndc, 0.5, 1.0);
    let dir = normalize(w.xyz / w.w);
    var col = sky_full(dir);
    if (camera_underwater()) {
        let wf = water_fog_view(dir * 512.0, frame.water_normal.w);
        col = col * wf.transmission + wf.color;
    }
    return vec4<f32>(col, 1.0);
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    return sky_pixel(in);
}

@fragment
fn fs_sky_g(in: SkyOut) -> GOut {
    var o: GOut;
    o.color = sky_pixel(in);
    o.gbuf = vec4<f32>(0.5, 0.5, 1.0, 0.0);
    return o;
}

// --------------------------------------------------------------- terrain

// Classic terrain (four detail textures). The PBR terrain has its own
// stage (TerrainPbrOut, vs_terrain_pbr, fs_terrain_pbr): one shader each, so
// that textured regions keep the cost of the classic one (its varyings,
// implicit-gradient samples and registers).
struct TerrainOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) comp: f32,
    @location(4) @interpolate(flat) record: u32,
};

struct TerrainPbrOut {
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) comp: f32,
    @location(4) @interpolate(flat) record: u32,
    @location(5) region_pos: vec3<f32>,
    @location(6) tangent0: vec4<f32>,
    @location(7) tangent1: vec4<f32>,
    @location(8) tangent2: vec4<f32>,
    @location(9) tangent3: vec4<f32>,
    @location(10) blend_noise: f32,
};

// textureUtilV.glsl terrain_tangent_space_transform, applied per vertex
// before interpolation, as pbrterrainV.glsl does for each of the four layers.
fn terrain_transform_tangent(rec: DrawRecord, n: vec3<f32>, raw: vec4<f32>) -> vec4<f32> {
    let scale_sign = sign(rec.uv_st.xy);
    var weights = vec2<f32>(cos(rec.params.x), sin(rec.params.x)) * scale_sign;
    weights.x += 1.0 - abs(sign(sign(weights.x) + 0.5 * sign(weights.y)));
    weights.y = -weights.y;
    let t = weights.x * raw.xyz + weights.y * raw.w * cross(n, raw.xyz);
    return vec4<f32>(normalize(t), raw.w * scale_sign.x * scale_sign.y);
}

@vertex
fn vs_terrain(in: VsIn) -> TerrainOut {
    let rec = records[in.instance];
    // The position of vs_depth, operation for operation: the depth prepass
    // draws the terrain with it, and without multisampling the scene pass
    // tests the terrain against that very depth. Computed in region
    // coordinates here, the depth differed by a rounding and patches of
    // terrain were rejected, showing the sky.
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: TerrainOut;
    out.clip = frame.eye_view_proj * ep;
    // (lighting, shadows and fog keep region coordinates)
    out.world_pos = ep.xyz + frame.eye.xyz;
    out.normal = vertex_normal(rec, in);
    // region metres; the composition in uv.x (world/terrain.rs build_chunk)
    out.uv = in.pos.xy;
    out.comp = in.uv.x;
    out.record = in.instance;
    return out;
}

@vertex
fn vs_terrain_pbr(in: VsIn) -> TerrainPbrOut {
    let rec = records[in.instance];
    // the position of vs_terrain, operation for operation (same prepass)
    let model = eye_model(rec, in);
    let ep = model * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: TerrainPbrOut;
    out.clip = frame.eye_view_proj * ep;
    out.world_pos = ep.xyz + frame.eye.xyz;
    out.normal = vertex_normal(rec, in);
    let raw = vec4<f32>(terrain_direction(vec4<u32>(round(in.weights * 255.0))), in.normal.w);
    out.tangent0 = terrain_transform_tangent(records[rec.tex.x], out.normal, raw);
    out.tangent1 = terrain_transform_tangent(records[rec.tex.y], out.normal, raw);
    out.tangent2 = terrain_transform_tangent(records[rec.tex.z], out.normal, raw);
    out.tangent3 = terrain_transform_tangent(records[rec.tex.w], out.normal, raw);
    out.uv = in.pos.xy;
    out.region_pos = in.pos;
    out.comp = in.uv.x;
    out.blend_noise = in.uv.y;
    out.record = in.instance;
    return out;
}

// IMG_ALPHA_GRAD (skins/default/textures/alpha_gradient.tga, Second Life
// viewer, Copyright (C) Linden Research, Inc., LGPL 2.1): 256 x 1 alpha
// ramp from 255 down to 0, slightly eased at both ends. Four 8-bit texels
// per word, first texel in the low byte.
const TERRAIN_ALPHA_RAMP: array<u32, 64> = array<u32, 64>(
    0xfdfdfeffu, 0xfbfbfbfcu, 0xf9f9fafau, 0xf6f6f7f7u, 0xf3f4f4f5u, 0xf0f1f1f2u, 0xedeeeeefu, 0xeaeaebecu,
    0xe6e7e7e8u, 0xe3e3e4e5u, 0xdfe0e1e2u, 0xdbdcdddfu, 0xd7d9d9dau, 0xd3d5d5d7u, 0xcfd0d2d3u, 0xcbcccdcfu,
    0xc7c8c9cau, 0xc2c3c5c6u, 0xbebfc0c1u, 0xb9babbbdu, 0xb5b6b7b9u, 0xb0b1b2b4u, 0xabacaeafu, 0xa7a8a9abu,
    0xa2a4a4a6u, 0x9d9ea0a1u, 0x989a9b9cu, 0x94959697u, 0x8f8f9193u, 0x8a8a8c8du, 0x84868788u, 0x80818283u,
    0x7b7b7d7eu, 0x7676787au, 0x70727374u, 0x6c6c6f6fu, 0x6768696au, 0x62636466u, 0x5d5e5f61u, 0x585a5b5cu,
    0x53545657u, 0x4f505152u, 0x4a4c4d4eu, 0x45474849u, 0x41424344u, 0x3d3d3f3fu, 0x38393a3bu, 0x34353537u,
    0x2f303232u, 0x2c2c2e2fu, 0x2728292bu, 0x23242527u, 0x20212223u, 0x1d1e1e1fu, 0x19191b1cu, 0x15161718u,
    0x12131415u, 0x0f101111u, 0x0c0d0d0fu, 0x090a0a0bu, 0x06070909u, 0x04050506u, 0x02020303u, 0x00010101u,
);

fn terrain_ramp_texel(i: u32) -> f32 {
    return f32((TERRAIN_ALPHA_RAMP[i >> 2u] >> ((i & 3u) * 8u)) & 0xffu) / 255.0;
}

// texture(alpha_ramp, vec2(u, ..)).a with the ramp's GL_LINEAR filtering
// and TAM_CLAMP addressing (texel centers at (i + 0.5) / 256).
fn terrain_ramp(u: f32) -> f32 {
    let x = clamp(u * 256.0 - 0.5, 0.0, 255.0);
    let i = u32(x);
    return mix(terrain_ramp_texel(i), terrain_ramp_texel(min(i + 1u, 255u)), x - f32(i));
}

// terrainF.glsl / terrainV.glsl layer weights for the composition value c
// in [0, 3]: mix(mix(d3, d2, R(c - 2)), mix(d1, d0, R(c)), R(c - 1)) with R
// the alpha ramp, written as one weight per detail texture.
fn terrain_weights(c: f32) -> vec4<f32> {
    let a1 = terrain_ramp(c);
    let a2 = terrain_ramp(c - 2.0);
    let af = terrain_ramp(c - 1.0);
    return vec4<f32>(af * a1, af * (1.0 - a1), (1.0 - af) * a2, (1.0 - af) * (1.0 - a2));
}

// LLDrawPoolTerrain binds IMG_ALPHA_GRAD_2D, not the one-dimensional ramp
// used by the old approximation above. Slot 6 is reserved by TextureTable.
// Its decoded image is top-down; LLSurfacePatch::eval supplies GL coordinates.
fn terrain_noise_weights(c: f32, noise: f32, dx: vec2<f32>, dy: vec2<f32>, clamp_sampler: sampler) -> vec4<f32> {
    let ramp_loc = tex_loc[6u];
    let uv = vec2<f32>(c, 1.0 - noise);
    let gx = dx * vec2<f32>(1.0, -1.0);
    let gy = dy * vec2<f32>(1.0, -1.0);
    let a = textureSampleGrad(tex_pages[ramp_loc >> 16u], clamp_sampler, uv, ramp_loc & 0xffffu, gx, gy).r;
    let b = textureSampleGrad(tex_pages[ramp_loc >> 16u], clamp_sampler, uv - vec2<f32>(2.0, 0.0), ramp_loc & 0xffffu, gx, gy).r;
    let f = textureSampleGrad(tex_pages[ramp_loc >> 16u], clamp_sampler, uv - vec2<f32>(1.0, 0.0), ramp_loc & 0xffffu, gx, gy).r;
    return vec4<f32>(f * a, f * (1.0 - a), (1.0 - f) * b, (1.0 - f) * (1.0 - b));
}

fn terrain_mix_weights(in: TerrainPbrOut, dx: vec3<f32>, dy: vec3<f32>, blend_dx: vec2<f32>, blend_dy: vec2<f32>, clamp_sampler: sampler) -> vec4<f32> {
    let rec = records[in.record];
    if (rec.flags.z != 0u) {
        // LLTerrainPaintMap stores slots 2..4 in linear RGB; slot 1 is the
        // remainder. No height/noise threshold is applied to a paintmap.
        let slot = rec.flags.z;
        let loc = tex_loc[slot];
        // Clamp each mip level at region edges; detail maps use repeat.
        let uv = in.region_pos.xy / rec.mat_uv.xy;
        var other = max(textureSampleGrad(tex_pages[loc >> 16u], clamp_sampler, uv, loc & 0xffffu, dx.xy / rec.mat_uv.xy, dy.xy / rec.mat_uv.xy).rgb, vec3<f32>(0.0));
        other /= max(1.0, dot(other, vec3<f32>(1.0)));
        return vec4<f32>(max(0.0, 1.0 - dot(other, vec3<f32>(1.0))), other);
    }
    let weights = max(terrain_noise_weights(clamp(in.comp, 0.0, 3.0), in.blend_noise, blend_dx, blend_dy, clamp_sampler) - vec4<f32>(0.1), vec4<f32>(0.0));
    return weights / dot(weights, vec4<f32>(1.0));
}

// LLDrawPoolTerrain::renderFullShaderPBR and pbrterrainUtilF.glsl:
// 8 m repeats, triplanar sharpness 8, material cutoff .1, plane cutoff .01.
// Shared material records keep the ordinary bindless texture table and make
// terrain edits independent of the chunk geometry. Explicit gradients let
// zero-weight layers/planes leave without sampling any maps.
struct TerrainMaterial {
    color: vec4<f32>,
    orm: vec3<f32>,
    normal: vec3<f32>,
    emissive: vec3<f32>,
    cutoff: f32,
};

fn terrain_uv(p: vec2<f32>, rec: DrawRecord) -> vec2<f32> {
    let v = p * rec.uv_st.xy;
    let c = cos(rec.params.x);
    let s = sin(rec.params.x);
    // terrain_texture_transform flips V before and after KHR's rotation.
    return vec2<f32>(c*v.x-s*v.y, s*v.x+c*v.y);
}

fn terrain_sample(slot: u32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let loc = tex_loc[slot];
    return textureSampleGrad(tex_pages[loc >> 16u], tex_sampler, uv, loc & 0xffffu, dx, dy);
}

fn terrain_layer(rec: DrawRecord, p: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, n: vec3<f32>, tangent: vec4<f32>, planes: vec3<f32>, alpha_only: bool) -> TerrainMaterial {
    var result = TerrainMaterial(vec4<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), rec.params.w);
    let tangent_sign = tangent.w;
    for (var axis = 0u; axis < 3u; axis++) {
        if (planes[axis] <= 0.0) { continue; }
        var uv = p.xy;
        var ux = dx.xy;
        var uy = dy.xy;
        if (axis == 0u) {
            let sign = select(-1.0, 1.0, n.x >= 0.0);
            uv = p.yz * vec2<f32>(sign, 1.0);
            ux = dx.yz * vec2<f32>(sign, 1.0);
            uy = dy.yz * vec2<f32>(sign, 1.0);
        } else if (axis == 1u) {
            let sign = select(1.0, -1.0, n.y >= 0.0);
            uv = p.xz * vec2<f32>(sign, 1.0);
            ux = dx.xz * vec2<f32>(sign, 1.0);
            uy = dy.xz * vec2<f32>(sign, 1.0);
        }
        let gl_tc = terrain_uv(uv, rec) + vec2<f32>(rec.uv_st.z, -rec.uv_st.w);
        let gl_tx = terrain_uv(ux, rec);
        let gl_ty = terrain_uv(uy, rec);
        // LLImageRaw is bottom-up; our decoded JPEG2000 rows are top-down.
        // Convert the completed GL transform, including offset and gradients.
        let tc = vec2<f32>(gl_tc.x, 1.0 - gl_tc.y);
        let tx = gl_tx * vec2<f32>(1.0, -1.0);
        let ty = gl_ty * vec2<f32>(1.0, -1.0);
        let color = terrain_sample(rec.tex.x, tc, tx, ty);
        result.color += vec4<f32>(srgb_to_linear(color.rgb), color.a) * rec.base_color * planes[axis];
        if (alpha_only) { continue; }
        let orm = terrain_sample(rec.tex.z, tc, tx, ty).rgb * vec3<f32>(1.0, rec.params.z, rec.params.y);
        let emission = srgb_to_linear(terrain_sample(rec.tex.w, tc, tx, ty).rgb) * rec.emissive.rgb;
        var mapped = vec3<f32>(0.0, 0.0, 1.0);
        if (rec.tex.y != 1u) {
            mapped = terrain_sample(rec.tex.y, tc, tx, ty).rgb * 2.0 - vec3<f32>(1.0);
        }
        // pbrterrainUtilF.glsl's axial normal orientation. Blend tangent-space
        // samples first, then apply the material's transformed Mikk basis once.
        mapped = vec3<f32>(mapped.xy * select(1.0, -1.0, n[axis] < 0.0), mapped.z);
        if (axis == 0u) { mapped = vec3<f32>(vec2<f32>(-mapped.y, mapped.x) * tangent_sign, mapped.z); }
        if (axis == 1u) { mapped = vec3<f32>(-mapped.xy, mapped.z); }
        result.orm += orm * planes[axis];
        result.normal += mapped * planes[axis];
        result.emissive += emission * planes[axis];
    }
    if (!alpha_only) {
        // MikkTSpace leaves T interpolated here; renormalizing it or deriving
        // it from N changes relief on curved terrain (pbrterrainF.glsl).
        let t = tangent.xyz;
        let b = tangent_sign * cross(n, t);
        result.normal = normalize(t * result.normal.x + b * result.normal.y + n * result.normal.z);
    }
    return result;
}

fn terrain_material(in: TerrainPbrOut, dx: vec3<f32>, dy: vec3<f32>, blend_dx: vec2<f32>, blend_dy: vec2<f32>, alpha_only: bool, clamp_sampler: sampler) -> TerrainMaterial {
    let rec = records[in.record];
    let n = normalize(in.normal);
    let tangents = array<vec4<f32>, 4>(in.tangent0, in.tangent1, in.tangent2, in.tangent3);
    var result = TerrainMaterial(vec4<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), 0.0);
    let weights = terrain_mix_weights(in, dx, dy, blend_dx, blend_dy, clamp_sampler);
    var planes = pow(abs(n), vec3<f32>(8.0));
    planes /= dot(planes, vec3<f32>(1.0));
    planes = max(planes - vec3<f32>(0.01), vec3<f32>(0.0));
    planes /= dot(planes, vec3<f32>(1.0));
    for (var layer = 0u; layer < 4u; layer++) {
        if (weights[layer] <= 0.0) { continue; }
        let layer_record = records[rec.tex[layer]];
        let tangent = tangents[layer];
        let sample = terrain_layer(layer_record, in.region_pos, dx, dy, n, tangent, planes, alpha_only);
        result.color += sample.color * weights[layer];
        result.orm += sample.orm * weights[layer];
        result.normal += sample.normal * weights[layer];
        result.emissive += sample.emissive * weights[layer];
        result.cutoff += sample.cutoff * weights[layer];
    }
    result.normal = normalize(result.normal + n * 0.000001);
    return result;
}

fn terrain_color(in: TerrainOut) -> vec3<f32> {
    let rec = records[in.record];
    // uv is region-space meters; detail textures repeat every uv_st.x meters
    let tc = in.uv / max(rec.uv_st.x, 0.1);
    let t0 = sample_tex(rec.tex.x, tc).rgb;
    let t1 = sample_tex(rec.tex.y, tc).rgb;
    let t2 = sample_tex(rec.tex.z, tc).rgb;
    let t3 = sample_tex(rec.tex.w, tc).rgb;
    let w = terrain_weights(clamp(in.comp, 0.0, 3.0));
    let w0 = w.x;
    let w1 = w.y;
    let w2 = w.z;
    let w3 = w.w;
    var albedo = srgb_to_linear(t0 * w0 + t1 * w1 + t2 * w2 + t3 * w3);
    if (rec.tex.x == 0u && rec.tex.y == 0u) {
        // no detail textures: flat palette (sand, grass, earth, rock)
        let p0 = vec3<f32>(0.62, 0.55, 0.40);
        let p1 = vec3<f32>(0.20, 0.32, 0.14);
        let p2 = vec3<f32>(0.30, 0.25, 0.18);
        let p3 = vec3<f32>(0.42, 0.42, 0.44);
        albedo = srgb_to_linear(p0 * w0 + p1 * w1 + p2 * w2 + p3 * w3) * 1.6;
    }
    var wet=0.0;
    if(water_fx_on(1024u)) {
        let level=water_fx_bed(in.world_pos.xy).y;
        wet=(1.0-smoothstep(0.05,0.45,in.world_pos.z-level))*smoothstep(-2.0,-0.1,in.world_pos.z-level);
        albedo*=1.0-wet*0.32;
    }
    let n = normalize(in.normal);
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let dist = distance(in.world_pos, frame.camera_pos.xyz);
    let sh = sun_shadow(in.world_pos, n, dist, in.clip.xy);
    var wet_light=vec3<f32>(0.0);
    if(wet>0.0) {
        let l=normalize(frame.sun_dir.xyz);let half_vec=normalize(l+v);
        let nv=max(dot(n,v),0.001);let nl=max(dot(n,l),0.0);
        let fresnel=0.02037+0.97963*pow(1.0-nv,5.0);
        let glint=d_ggx(max(dot(n,half_vec),0.0),0.08)*v_smith(nv,nl,0.08)*nl*0.02;
        wet_light=(sky_color(reflect(-v,n))*fresnel*0.25+frame.sun_color.rgb*min(glint,2.0)*sh)*wet;
    }
    if (is_classic_sky()) {
        let a = ll_atmos(in.world_pos - frame.camera_pos.xyz);
        let hazed = ll_haze(ll_classic_diffuse(albedo, n, in.world_pos, sh, in.clip.xy, a) + wet_light + albedo*water_caustics(in.world_pos,n), a);
        var color=hazed*water_scene_light(in.world_pos);
        if(camera_underwater() || (water_fx_on(256u) && in.world_pos.z<camera_water_level())){color=apply_fog(color,in.world_pos);}
        return light_tiles_checked(debug_view(color, in.world_pos, n, dist, in.clip.xy), in.world_pos, in.clip.xy);
    }
    let col = shade_pbr(albedo, 0.0, mix(0.92,0.4,wet), 1.0, n, v, in.world_pos, sh, in.clip.xy, true) + wet_light + albedo*water_caustics(in.world_pos,n);
    return light_tiles_checked(debug_view(apply_fog(col*water_scene_light(in.world_pos), in.world_pos), in.world_pos, n, dist, in.clip.xy), in.world_pos, in.clip.xy);
}

fn terrain_pbr_shaded(in: TerrainPbrOut) -> Shaded {
    let dx = dpdx(in.region_pos);
    let dy = dpdy(in.region_pos);
    let blend_dx = dpdx(vec2<f32>(in.comp, in.blend_noise));
    let blend_dy = dpdy(vec2<f32>(in.comp, in.blend_noise));
    let geo = normalize(in.normal);
    let material = terrain_material(in, dx, dy, blend_dx, blend_dy, false, lin_clamp);
    surface_occlusion = normal_map_occlusion(geo, material.normal, in.world_pos, 1.0 / in.clip.w);
    screen_surface = true;
    contact_surface = true;
    if (material.color.a < material.cutoff) { discard; }
    let n = material.normal;
    let v = normalize(frame.camera_pos.xyz - in.world_pos);
    let dist = distance(in.world_pos, frame.camera_pos.xyz);
    let sh = sun_shadow(in.world_pos, geo, dist, in.clip.xy);
    let metal = clamp(material.orm.b, 0.0, 1.0);
    // pbrterrainF.glsl writes the material's ORM unchanged. Broadening its
    // roughness from normal derivatives washes out terrain relief.
    let rough = clamp(material.orm.g, 0.03, 1.0);
    let ao = select(1.0, material.orm.r, frame.sky_ll.z > 0.5);
    // pbrterrainF.glsl writes its emissive only with HAS_EMISSIVE
    let emissive = select(material.emissive, vec3<f32>(0.0), deferred_emissive_off());
    var color: vec3<f32>;
    if (is_classic_sky()) {
        let a = ll_atmos(in.world_pos - frame.camera_pos.xyz);
        color = (ll_classic_pbr(material.color.rgb, metal, rough, ao, n, v, in.world_pos, sh, in.clip.xy, a)
            + emissive) * ll_final_scale()
            + ll_classic_pbr_local(material.color.rgb, metal, rough, n, v, in.world_pos, in.clip.xy);
        color = ll_haze(color + material.color.rgb * water_caustics(in.world_pos, n), a);
        if (camera_underwater() || (water_fx_on(256u) && in.world_pos.z < camera_water_level())) {
            color = apply_fog(color * water_scene_light(in.world_pos), in.world_pos);
        }
    } else {
        color = shade_pbr(material.color.rgb, metal, rough, ao, n, v, in.world_pos, sh, in.clip.xy, false) + emissive;
        color += material.color.rgb * water_caustics(in.world_pos, n);
        color = apply_fog(color * water_scene_light(in.world_pos), in.world_pos);
    }
    color = debug_view(color, in.world_pos, n, dist, in.clip.xy);
    return Shaded(vec4<f32>(light_tiles_checked(color, in.world_pos, in.clip.xy), 1.0), n, rough, metal);
}

@fragment
fn fs_terrain(in: TerrainOut) -> @location(0) vec4<f32> {
    let col = terrain_color(in);
    if (reflection_clip(in.world_pos)) {
        discard;
    }
    return vec4<f32>(col, 1.0);
}

@fragment
fn fs_terrain_g(in: TerrainOut) -> GOut {
    var o: GOut;
    o.color = vec4<f32>(terrain_color(in), 1.0);
    o.gbuf = vec4<f32>(oct_encode(normalize(in.normal)), 1.0, 0.0);
    return o;
}

@fragment
fn fs_terrain_pbr(in: TerrainPbrOut) -> @location(0) vec4<f32> {
    let shaded = terrain_pbr_shaded(in);
    if (reflection_clip(in.world_pos)) {
        discard;
    }
    return shaded.color;
}

@fragment
fn fs_terrain_pbr_g(in: TerrainPbrOut) -> GOut {
    var o: GOut;
    let shaded = terrain_pbr_shaded(in);
    o.color = shaded.color;
    o.gbuf = gbuffer(shaded);
    return o;
}

// Match the main terrain mask before writing either single-sample depth or
// MSAA depth. Alpha-only sampling skips ORM, normal and emissive maps.
fn terrain_depth_mask(in: TerrainPbrOut) {
    let dx = dpdx(in.region_pos);
    let dy = dpdy(in.region_pos);
    let blend_dx = dpdx(vec2<f32>(in.comp, in.blend_noise));
    let blend_dy = dpdy(vec2<f32>(in.comp, in.blend_noise));
    if ((records[in.record].flags.x & FLAG_ALPHA_MASK) != 0u) {
        let material = terrain_material(in, dx, dy, blend_dx, blend_dy, true, lin_clamp);
        if (material.color.a < material.cutoff) { discard; }
    }
}

// Terrain shadow casters (the renderer's `shadow_terrain` pipeline, when not
// drawn depth only): vs_shadow's placement plus the terrain's inputs, so
// that the other casters keep vs_shadow / fs_shadow without them.
struct TerrainShadowOut {
    @builtin(position) clip: vec4<f32>,
    @location(1) @interpolate(flat) record: u32,
    @location(2) world_pos: vec3<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) region_pos: vec3<f32>,
    @location(5) comp: f32,
    @location(6) blend_noise: f32,
};

@vertex
fn vs_shadow_terrain(in: VsIn) -> TerrainShadowOut {
    let rec = records[in.instance];
    let wp = skinned_model(rec, in) * vec4<f32>(vertex_position(rec, in), 1.0);
    var out: TerrainShadowOut;
    out.clip = shadow_params.vp * wp;
    out.record = in.instance;
    out.world_pos = wp.xyz;
    out.normal = vertex_normal(rec, in);
    out.region_pos = in.pos;
    out.comp = in.uv.x;
    out.blend_noise = in.uv.y;
    return out;
}

// Sun cascades and projector shadows use the same region coordinates,
// composition and triplanar alpha as colour, depth and picking. Derivatives
// precede the material branch so neighbouring fragments can use other modes.
fn terrain_shadow_mask(in: TerrainShadowOut) {
    let dx = dpdx(in.region_pos);
    let dy = dpdy(in.region_pos);
    let blend_dx = dpdx(vec2<f32>(in.comp, in.blend_noise));
    let blend_dy = dpdy(vec2<f32>(in.comp, in.blend_noise));
    if ((records[in.record].flags.x & (FLAG_TERRAIN | FLAG_PBR | FLAG_ALPHA_MASK)) == (FLAG_TERRAIN | FLAG_PBR | FLAG_ALPHA_MASK)) {
        var surface: TerrainPbrOut;
        surface.normal = in.normal;
        surface.region_pos = in.region_pos;
        surface.comp = in.comp;
        surface.blend_noise = in.blend_noise;
        surface.record = in.record;
        let material = terrain_material(surface, dx, dy, blend_dx, blend_dy, true, shadow_clamp);
        if (material.color.a < material.cutoff) { discard; }
    }
}

@fragment
fn fs_shadow_terrain(in: TerrainShadowOut) {
    terrain_shadow_mask(in);
    portal_clip(records[in.record], in.world_pos);
}

// Masked PBR terrain only: the other terrain is drawn by vs_depth.
@fragment
fn fs_terrain_depth(in: TerrainPbrOut) {
    terrain_depth_mask(in);
}

@fragment
fn fs_terrain_depth_normal(in: TerrainPbrOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    terrain_depth_mask(in);
    return depth_normal(in.normal, front);
}

// Picking gives the region metres as the surface coordinates of a hit (the
// vertex uv holds the composition): classic and opaque PBR terrain.
@fragment
fn fs_terrain_pick(in: TerrainOut) -> @location(0) vec4<u32> {
    return vec4<u32>(in.record + 1u, bitcast<u32>(in.clip.z), bitcast<u32>(in.uv.x), bitcast<u32>(in.uv.y));
}

// Masked PBR terrain: its holes are no targets.
@fragment
fn fs_terrain_pick_mask(in: TerrainPbrOut) -> @location(0) vec4<u32> {
    terrain_depth_mask(in);
    return vec4<u32>(in.record + 1u, bitcast<u32>(in.clip.z), bitcast<u32>(in.uv.x), bitcast<u32>(in.uv.y));
}

// ----------------------------------------------------------------- water

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) level: f32,
};

fn water_vertex(in: VsIn) -> WaterOut {
    let rec = records[in.instance];
    var wp = rec.model * vec4<f32>(in.pos, 1.0);
    var wave = vec3<f32>(0.0);
    if(water_fx_on(1u) || water_fx_on(16u)){wave=water_fx_wave(wp.xy,wp.z);}
    // Huge void tiles cannot resolve waves. Fade their geometry to avoid aliasing.
    let tile = max(length(rec.model[0].xyz),length(rec.model[1].xyz));
    let geometry = 1.0-smoothstep(70.0,140.0,tile);
    let near = 1.0-smoothstep(80.0,160.0,distance(wp.xyz,frame.camera_pos.xyz));
    wp = vec4<f32>(wp.xyz+wave*geometry*near,1.0);
    var out: WaterOut;
    out.clip = frame.view_proj * wp;
    out.world_pos = wp.xyz;
    out.level = (rec.model * vec4<f32>(in.pos,1.0)).z;
    return out;
}

@vertex
fn vs_water(in:VsIn)->WaterOut { return water_vertex(in); }

@vertex
fn vs_water_unclipped(in:VsIn)->WaterOut {
    var out=water_vertex(in);
    // Preserve XY/W and perspective interpolation of the actual surface.
    // Only raster depth changes; the fragment restores its physical depth.
    out.clip.z=out.clip.w*0.5;
    return out;
}

fn analytic_wave(p: vec2<f32>, t: f32) -> vec3<f32> {
    var g = vec2<f32>(0.0);
    let dirs = array<vec2<f32>, 4>(
        vec2<f32>(0.8, 0.6), vec2<f32>(-0.6, 0.8), vec2<f32>(0.3, -0.95), vec2<f32>(-0.9, -0.35));
    let freqs = array<f32, 4>(0.35, 0.6, 1.3, 2.1);
    let amps = array<f32, 4>(0.12, 0.07, 0.035, 0.02);
    let speeds = array<f32, 4>(0.9, 1.3, 1.9, 2.6);
    for (var i = 0; i < 4; i++) {
        let ph = dot(dirs[i], p) * freqs[i] + t * speeds[i];
        g += dirs[i] * cos(ph) * freqs[i] * amps[i];
    }
    return normalize(vec3<f32>(-g.x, -g.y, 1.0));
}

// Reuse opaque depth/color: no second scene render or GPU/CPU synchronization.
fn water_source_fog(source:vec3<f32>)->WaterFog {
    if(!water_fx_on(256u) && !camera_underwater()){return WaterFog(vec3<f32>(0.0),vec3<f32>(1.0));}
    return water_fog_view(source-frame.camera_pos.xyz,frame.water_normal.w);
}

// Rays cover the same 200 m at every quality, with finer steps at high quality.
// Missing/offscreen geometry falls back to the environment, as with water SSR.
fn water_scene_ray(origin:vec3<f32>,direction:vec3<f32>,level:f32,above:bool)->vec4<f32> {
    let steps=12u+water_fx.counts.w*6u;
    let growth=pow(2000.0,1.0/f32(steps));
    var t=0.1*growth;var previous=0.0;
    for(var i=0u;i<steps;i++) {
        let p=origin+direction*t;
        let clip=frame.view_proj*vec4<f32>(p,1.0);
        if(clip.w<=0.01){break;}
        let ndc=clip.xyz/clip.w;
        let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
        if(any(uv<vec2<f32>(0.001)) || any(uv>vec2<f32>(0.999))){break;}
        let depth=scene_depth_at(uv);
        if(depth>0.0 && depth>ndc.z) {
            let hit=world_from_depth(uv,depth);
            let gap=distance(hit,frame.camera_pos.xyz)-distance(p,frame.camera_pos.xyz);
            if(abs(gap)<max(t-previous,0.15)*1.5) {
                var lo=previous;var hi=t;var hit_uv=uv;
                for(var k=0u;k<4u;k++) {
                    let mid=(lo+hi)*0.5;
                    let c=frame.view_proj*vec4<f32>(origin+direction*mid,1.0);
                    let q=c.xyz/c.w;
                    let u=vec2<f32>(q.x*0.5+0.5,0.5-q.y*0.5);
                    if(scene_depth_at(u)>q.z){hi=mid;hit_uv=u;}else{lo=mid;}
                }
                let source=world_from_depth(hit_uv,scene_depth_at(hit_uv));
                if(select(source.z>level+0.04,source.z<level-0.04,above)){return vec4<f32>(0.0);}
                // Terrain has a continuous bathymetric fallback below. Sparse
                // screen-space hits must not paint alternating bright/dark bands.
                if(!above && source.z<water_fx_bed(source.xy).x+0.35){return vec4<f32>(0.0);}
                let source_fog=water_source_fog(source);
                let sampled=textureSampleLevel(scene_tex,lin_clamp,hit_uv,0.0).rgb;
                // The source was already fogged in the opaque pass. Recover its
                // incident radiance before applying the actual surface path once.
                var color=max(sampled-source_fog.color,vec3<f32>(0.0))/max(source_fog.transmission,vec3<f32>(0.05));
                if(!above) {
                    let reflected_fog=water_fog_segment(source-origin,frame.water_normal.w,max(level-origin.z,0.0),max(level-source.z,0.0));
                    color=color*reflected_fog.transmission+reflected_fog.color;
                }
                let edge=min(min(hit_uv.x,1.0-hit_uv.x),min(hit_uv.y,1.0-hit_uv.y));
                let confidence=smoothstep(0.0,0.035,edge)*smoothstep(0.03,0.12,max(max(source_fog.transmission.r,source_fog.transmission.g),source_fog.transmission.b));
                return vec4<f32>(color,confidence);
            }
        }
        previous=t;t*=growth;
    }
    return vec4<f32>(0.0);
}

fn water_bottom_reflection(surface:vec3<f32>,direction:vec3<f32>,level:f32)->vec3<f32> {
    // Approximate the rough, multiple-bounce floor lobe continuously when a
    // micro-normal sends the single reflected ray back toward the underside.
    let floor_ray=normalize(vec3<f32>(direction.xy,-sqrt(direction.z*direction.z+0.0144)));
    var t=clamp((water_fx_bed(surface.xy).x-surface.z)/floor_ray.z,0.05,200.0);
    for(var i=0u;i<2u;i++) {
        let p=surface+floor_ray*t;
        t=clamp((water_fx_bed(p.xy).x-surface.z)/floor_ray.z,0.05,200.0);
    }
    let bottom=surface+floor_ray*t;
    let c=frame.view_proj*vec4<f32>(bottom,1.0);
    if(c.w<=0.01){return frame.water_fog.rgb;}
    // Outside the image, extend blurred bottom radiance rather than inventing
    // sharp offscreen geometry or falling through to black gaps between SSR hits.
    let uv=clamp(vec2<f32>(c.x/c.w*0.5+0.5,0.5-c.y/c.w*0.5),vec2<f32>(0.015),vec2<f32>(0.985));
    let depth=scene_depth_at(uv);
    if(depth<=0.0){return frame.water_fog.rgb;}
    let source=world_from_depth(uv,depth);
    if(source.z>level-0.03){return frame.water_fog.rgb;}
    let sampled=(textureSampleLevel(scene_tex,lin_clamp,uv,0.0).rgb
        +textureSampleLevel(scene_tex,lin_clamp,uv+vec2<f32>(0.006,0.0),0.0).rgb
        +textureSampleLevel(scene_tex,lin_clamp,uv-vec2<f32>(0.006,0.0),0.0).rgb
        +textureSampleLevel(scene_tex,lin_clamp,uv+vec2<f32>(0.0,0.006),0.0).rgb
        +textureSampleLevel(scene_tex,lin_clamp,uv-vec2<f32>(0.0,0.006),0.0).rgb)*0.2;
    let source_fog=water_source_fog(source);
    let incoming=max(sampled-source_fog.color,vec3<f32>(0.0))/max(source_fog.transmission,vec3<f32>(0.05));
    let fog=water_fog_segment(floor_ray*t,frame.water_normal.w,0.0,max(level-bottom.z,0.0));
    return incoming*fog.transmission+fog.color;
}

// Exterior geometry has its own wider view: refracted rays commonly reach
// objects outside the main image. Two depth-guided steps correct near parallax.
fn water_exterior(surface:vec3<f32>,direction:vec3<f32>)->vec4<f32> {
    var point=surface+direction*200.0;
    var uv=vec2<f32>(0.5);
    for(var i=0u;i<3u;i++) {
        let clip=frame.water_air_vp*vec4<f32>(point,1.0);
        if(clip.w<=0.0001){return vec4<f32>(0.0);}
        uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5;
        if(any(uv<vec2<f32>(0.001)) || any(uv>vec2<f32>(0.999))){return vec4<f32>(0.0);}
        let size=vec2<i32>(textureDimensions(water_air_depth));
        let depth=textureLoad(water_air_depth,vec2<i32>(uv*vec2<f32>(size)),0);
        if(depth<=0.0){break;}
        let world=frame.water_air_inv_vp*vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),depth,1.0);
        let source=world.xyz/world.w;
        let along=dot(source-surface,direction);
        if(along<=0.0){return vec4<f32>(0.0);}
        point=surface+direction*along;
    }
    let edge=min(min(uv.x,1.0-uv.x),min(uv.y,1.0-uv.y));
    return vec4<f32>(textureSampleLevel(water_air_color,lin_clamp,uv,0.0).rgb,smoothstep(0.0,0.025,edge));
}

fn water_dielectric(cos_i:f32)->f32 {
    let eta=1.333;
    let sin_t2=eta*eta*(1.0-cos_i*cos_i);
    if(sin_t2>=1.0){return 1.0;}
    let cos_t=sqrt(max(1.0-sin_t2,0.0));
    let rs=(eta*cos_i-cos_t)/max(eta*cos_i+cos_t,0.001);
    let rp=(cos_i-eta*cos_t)/max(cos_i+eta*cos_t,0.001);
    return (rs*rs+rp*rp)*0.5;
}

fn water_underside(surface:vec3<f32>,level:f32,incident:vec3<f32>,normal:vec3<f32>)->vec3<f32> {
    let fog=water_fog_segment(surface-frame.camera_pos.xyz,frame.water_normal.w,max(camera_water_level()-frame.camera_pos.z,0.0),0.0);
    if(!water_fx_on(512u)) {
        // The disabled/master-off path keeps inexpensive baseline transparency.
        let clip=frame.view_proj*vec4<f32>(surface,1.0);
        let ndc=clip.xy/clip.w;
        let uv=vec2<f32>(ndc.x*0.5+0.5,0.5-ndc.y*0.5);
        var distorted=clamp(uv+normal.xy*0.008,vec2<f32>(0.001),vec2<f32>(0.999));
        var depth=scene_depth_at(distorted);
        if(depth>0.0 && world_from_depth(distorted,depth).z<level-0.04){distorted=uv;depth=scene_depth_at(uv);}
        var color=sky_full(incident);
        if(depth>0.0) {
            let source=world_from_depth(distorted,depth);
            let source_fog=water_source_fog(source);
            let sampled=textureSampleLevel(scene_tex,lin_clamp,distorted,0.0).rgb;
            color=max(sampled-source_fog.color,vec3<f32>(0.0))/max(source_fog.transmission,vec3<f32>(0.05));
        }
        return color*fog.transmission+fog.color;
    }
    let eta=1.333;
    let cos_i=clamp(dot(-normal,-incident),0.0,1.0);
    var reflected=frame.water_fog.rgb;
    var transmitted=vec3<f32>(0.0);
    let reflectance=water_dielectric(cos_i);
    if(reflectance<1.0) {
        let direction=refract(incident,-normal,eta);
        transmitted=sky_full(direction);
        var hit=vec4<f32>(0.0);
        if(frame.water_air.x>0.5){hit=water_exterior(surface,direction);}
        else {hit=water_scene_ray(surface+direction*0.025,direction,level,true);}
        transmitted=mix(transmitted,hit.rgb,hit.a);
    }
    if(reflectance>0.04) {
        let direction=reflect(incident,normal);
        reflected=water_bottom_reflection(surface,direction,level);
        let hit=water_scene_ray(surface+direction*0.025,direction,level,false);
        reflected=mix(reflected,hit.rgb,hit.a);
    }
    return mix(transmitted,reflected,reflectance)*fog.transmission+fog.color;
}

fn water_pixel(in: WaterOut) -> vec4<f32> {
    if(water_excluded(in.world_pos)){discard;}
    if(water_fx_on(1u) || water_fx_on(16u)) {
        let depth=water_interface_depth(in.world_pos);
        // Single-sample interface coverage must also retain MSAA edge samples.
        let tolerance=0.6*(abs(dpdx(depth))+abs(dpdy(depth)))+0.000001;
        let first=textureLoad(water_interface,vec2<i32>(in.clip.xy),0);
        if(depth+tolerance<first){discard;}
    }
    let uv = screen_uv(in.clip.xy);
    let t = frame.camera_pos.w * 0.5;
    let rel = in.world_pos - frame.camera_pos.xyz;
    let dist = length(rel);
    let view_vec = rel / max(dist, 1e-4);

    // SL wave layers (big wave + two little waves) from the normal map
    var v = in.world_pos.xy;
    v.x += (cos(v.x * 0.08) + sin(v.y * 0.02)) * 6.0;
    let w1d = frame.water_waves.xy;
    let w2d = frame.water_waves.zw;
    let big = v * 0.04 + w1d * t * 0.055;
    let little1 = v * vec2<f32>(0.45, 0.9) + w2d * t * 0.13;
    let little2 = v * vec2<f32>(0.1, 0.2) + w1d * t * 0.1;
    let slot = frame.tex_slots.w;
    var wave1 = sample_tex(slot, big).xyz * 2.0 - vec3<f32>(1.0);
    var wave2 = sample_tex(slot, little1).xyz * 2.0 - vec3<f32>(1.0);
    var wave3 = sample_tex(slot, little2).xyz * 2.0 - vec3<f32>(1.0);
    if (slot == 0u) {
        wave1 = analytic_wave(in.world_pos.xy, t * 2.0);
        wave2 = wave1;
        wave3 = wave1;
    }
    let wavef = (wave1 + wave2 * 0.4 + wave3 * 0.6) * 0.5;
    var n = wavef * frame.water_normal.xyz;
    n.z *= 2.0;
    let footprint=max(length(dpdx(in.world_pos.xy)),length(dpdy(in.world_pos.xy)));
    let wake=water_wake_sample(in.world_pos.xy,in.level);
    let furrow=1.0-smoothstep(0.003,0.035,-wake.z)*0.7;
    if(water_fx_on(1u)) {
        let micro=water_micro(in.world_pos.xy,frame.camera_pos.w,water_fx.counts.w,footprint);
        let detail=clamp(length(frame.water_normal.xy)*0.35,0.5,1.5);
        n=normalize(vec3<f32>((n.xy*0.18-micro.xy*detail)*furrow,1.0)+water_fx_normal(in.world_pos.xy,in.level,wake.xy));
    } else { n=normalize(n+water_fx_normal(in.world_pos.xy,in.level,wake.xy)); }

    // Below water, transmit the world through the moving Snell window and
    // reflect the submerged scene at grazing angles. The sky itself is unchanged.
    let geometric=cross(dpdx(in.world_pos),dpdy(in.world_pos));
    let up=geometric*select(-1.0,1.0,geometric.z>=0.0);
    let below=dot(frame.camera_pos.xyz-in.world_pos,up)<0.0;
    if(below) {
        return vec4<f32>(water_underside(in.world_pos,in.level,view_vec,n),1.0);
    }

    // fresnel (calculateFresnelFactors)
    let fs = frame.water_params.y;
    let fo = frame.water_params.z;
    var df3 = max(vec3<f32>(0.0), vec3<f32>(
        dot(view_vec, wave1), dot(view_vec, (wave2 + wave3) * 0.5), dot(view_vec, wave3)) * fs + vec3<f32>(fo));
    df3 *= df3;
    let df2 = max(vec2<f32>(0.0), vec2<f32>(df3.x + df3.y + df3.z, dot(view_vec, normalize(wavef)) * fs + fo));

    // refraction of the scene behind the surface
    let dmod = sqrt(dist);
    let waver = wavef * 3.0;
    let ref_scale = 0.018;
    var distortion=waver.xy*ref_scale/max(dmod,1.0)*2.0;
    if(water_fx_on(1u) || water_fx_on(16u)){distortion=n.xy*0.08/max(dmod,1.0);}
    var distort2=clamp(uv+distortion,vec2<f32>(0.0),vec2<f32>(0.999));
    let d0 = scene_depth_at(uv);
    let surf_dist = dist;
    var refr_dist = 1e5;
    if (d0 > 0.0) {
        refr_dist = distance(world_from_depth(uv, d0), frame.camera_pos.xyz);
    }
    // shoreline: no distortion where the water is shallow
    let fade = clamp((refr_dist - surf_dist) / 10.0, 0.0, 1.0);
    if(water_fx_on(1u) || water_fx_on(16u)) {
        let water_path=clamp(refr_dist-surf_dist,0.0,30.0);
        distort2=clamp(uv+n.xy*min(water_path*0.018,0.16)/max(dmod*0.5,1.0),vec2<f32>(0.0),vec2<f32>(0.999));
    }
    distort2 = mix(uv, distort2, min(1.0, fade * 10.0));
    let d1 = scene_depth_at(distort2);
    var ref_pos = frame.camera_pos.xyz + view_vec * 1e5;
    if (d1 > 0.0) {
        ref_pos = world_from_depth(distort2, d1);
    }
    if (distance(ref_pos, frame.camera_pos.xyz) < surf_dist - 0.05) {
        // the distorted sample is in front of the water: don't refract it
        distort2 = uv;
        ref_pos = frame.camera_pos.xyz + view_vec * refr_dist;
    }
    var fb = textureSampleLevel(scene_tex, lin_clamp, distort2, 0.0).rgb;
    // Fog of the water volume between the surface and the refracted point.
    let wf = water_fog_view(ref_pos - frame.camera_pos.xyz, frame.water_fog.w);
    if (water_fx_on(256u)) {
        let source_fog=water_source_fog(ref_pos);
        fb=max(fb-source_fog.color,vec3<f32>(0.0))/max(source_fog.transmission,vec3<f32>(0.05));
        let volume=water_fog_segment(ref_pos-in.world_pos,frame.water_normal.w,0.0,max(in.level-ref_pos.z,0.0));
        fb=fb*volume.transmission+volume.color;
    } else { fb = fb * wf.transmission + wf.color; }

    // reflection: planar texture, screen-space rays, or the sky
    let r = reflect(view_vec, normalize(mix(vec3<f32>(0.0, 0.0, 1.0), n, 0.6)));
    var radiance = sky_color(vec3<f32>(r.x, r.y, abs(r.z)));
    if (probes_on()) {
        // without a planar reflection the water reflects its probes
        let s = sample_probes_water(in.world_pos, in.clip.xy, vec3<f32>(r.x, r.y, abs(r.z)), 0.0);
        if (s.w > 0.0) {
            radiance = s.rgb;
        }
    }
    if (frame.misc.z > 0.5) {
        let ruv = clamp(uv + n.xy * 0.035 / max(dmod * 0.25, 1.0), vec2<f32>(0.001), vec2<f32>(0.999));
        radiance = textureSampleLevel(water_refl_tex, lin_clamp, ruv, 0.0).rgb;
    } else if (frame.misc.y > 0.5) {
        let hit = ssr_trace(in.world_pos + vec3<f32>(0.0, 0.0, 0.02), r, select(40,16 + i32(water_fx.counts.w) * 12,water_fx.counts.z!=0u));
        radiance = mix(radiance, hit.rgb, hit.a);
    }
    if (!water_fx_on(512u)) { radiance *= df2.y; }

    // sun glint (punctual light, roughness = blur multiplier)
    let l = normalize(frame.sun_dir.xyz);
    let vv = -view_vec;
    let h = normalize(l + vv);
    var rough=clamp(frame.water_params.w,0.02,1.0);
    if(water_fx_on(1u)){rough=clamp(rough*0.65+footprint*0.06,0.035,0.25);}
    rough=mix(rough,0.055,smoothstep(0.004,0.045,-wake.z)*0.65);
    let nh = max(dot(n, h), 0.0);
    let nl = max(dot(n, l), 0.0);
    let spec = d_ggx(nh, rough * rough) * v_smith(max(dot(n, vv), 1e-3), nl, rough * rough) * nl;
    let shadow = shadow_factor(in.world_pos, vec3<f32>(0.0, 0.0, 1.0), dist);
    // a sky with a probe ambiance: waterF.glsl's sunlit_linear (env.rs keeps
    // it x 3 for the surfaces)
    let sun_light = select(frame.sun_color.rgb, frame.sun_color.rgb / 3.0, frame.sky_ll.x < 0.5);
    let punctual = clamp(spec * sun_light * shadow, vec3<f32>(0.0), vec3<f32>(10.0));

    var reflectance = min(1.0,df2.x);
    var glint = punctual;
    if (water_fx_on(512u)) {
        let cos_i = clamp(dot(n,vv),0.0,1.0);
        reflectance = 0.02037+0.97963*pow(1.0-cos_i,5.0);
        glint *= 0.02037+0.97963*pow(1.0-max(dot(vv,h),0.0),5.0);
    }
    var color = mix(fb, radiance, reflectance) + glint;
    color = mix(fb, color, min(1.0, fade * 60.0));
    if((water_fx.counts.z &14u)!=0u) {
        let p=in.world_pos.xy;let bed=water_fx_bed(p);let depth=max(in.level-bed.x,0.0);
        let field=water_fx_sample(p);
        // Shore depth always comes from terrain, never an avatar or submerged
        // mesh in the opaque depth. Contact refinement is masked below.
        var foam=smoothstep(0.3,0.95,water_fx_foam(p))*0.55;
        if(water_fx_on(2u) && depth<2.0) {
            // Travel along the depth gradient: the front approaches every coast orientation.
            let phase=depth*3.4+frame.camera_pos.w*1.25+water_fx_noise(p*0.32)*1.1;
            let breaker=pow(max(sin(phase),0.0),4.0);
            let runup=0.12+0.28*sin(frame.camera_pos.w*0.6+water_fx_noise(p*0.15));
            let front=exp(-pow((depth-0.18-runup)*5.0,2.0));
            foam=max(foam,front*breaker*0.95);
            foam=max(foam,breaker*exp(-pow((depth-0.7)*2.8,2.0))*0.65);
        }
        if(water_fx_on(4u) && d0>0.0 && field.w>0.02) {
            let hit=world_from_depth(uv,d0);
            let ground=water_fx_bed(hit.xy).x;
            let clearance=in.level-hit.z;
            // The visible intersection refines the proxy to the actual mesh silhouette.
            let obstacle=smoothstep(0.18,0.55,hit.z-ground);
            foam+=obstacle*(1.0-smoothstep(0.03,0.42,clearance))*smoothstep(-0.08,0.03,clearance)*0.45*field.w;
        }
        if(water_fx_on(8u)) {
            let wave=water_swell(p,frame.camera_pos.w,water_fx.counts.w);
            let windpatch=water_fx_noise(p*0.22-frame.camera_pos.w*0.04);
            foam+=smoothstep(0.08,0.2,wave.compression)*smoothstep(0.45,0.8,windpatch)*0.6;
        }
        foam=water_foam_coverage(p,foam,footprint);
        // Fine breaking filaments already have a narrow, resolved footprint.
        // Applying the coarse accumulation threshold twice erased slow swimmers.
        if(water_fx_on(4u)){foam+=smoothstep(0.018,0.18,wake.w)*water_foam_coverage(p,1.0,footprint)*0.55;}
        foam=clamp(foam,0.0,1.0)*smoothstep(0.0,0.06,depth);
        let nl=max(frame.sun_dir.z,0.0);
        var foam_light=(frame.sky_horizon.rgb*0.3+frame.sun_color.rgb*nl*shadow/PI)*0.75;
        if(is_classic_sky()) {
            // Classic water uses the sky's radiance units. Limit highlights
            // without shifting their hue or inheriting the seabed's GTAO.
            let lit=(frame.sky_horizon.rgb*0.38+frame.sun_color.rgb*nl*shadow*0.7)*0.85;
            let peak=max(lit.r,max(lit.g,lit.b));
            foam_light=lit/max(1.0,peak/1.15);
        }
        color=mix(color,max(foam_light,vec3<f32>(0.0)),foam*0.9);
    }
    // The upper face is viewed through air. Opaque source water fog was
    // removed above; do not add another fictitious submerged interval here.
    if(camera_underwater()) {
        let fog=water_fog_view(rel,frame.water_normal.w);
        color=color*fog.transmission+fog.color;
    } else {color=apply_air_fog(color,in.world_pos);}
    return vec4<f32>(color, 1.0);
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    return water_pixel(in);
}

struct WaterDepthOut {
    @location(0) color:vec4<f32>,
    @builtin(frag_depth) depth:f32,
};

fn water_physical_depth(world:vec3<f32>)->f32 {
    let clip=frame.view_proj*vec4<f32>(world,1.0);
    return clamp(clip.z/max(clip.w,0.000001),0.0,1.0);
}

@fragment
fn fs_water_unclipped(in:WaterOut)->WaterDepthOut {
    return WaterDepthOut(water_pixel(in),water_physical_depth(in.world_pos));
}

fn water_interface_depth(world:vec3<f32>)->f32 {
    // Standard near-clamped depth merges all interfaces before near. This
    // ordering retains their true distance, including millimetre crossings.
    let clip=frame.view_proj*vec4<f32>(world,1.0);
    return 1.0/(1.0+max(clip.w,0.0));
}

struct WaterMaskOut {
    @location(0) color:vec4<f32>,
    @builtin(frag_depth) depth:f32,
};

@fragment
fn fs_water_mask(in:WaterOut)->WaterMaskOut {
    return WaterMaskOut(vec4<f32>(1.0),water_interface_depth(in.world_pos));
}

@fragment
fn fs_water_exclusion(in:VsOut)->WaterMaskOut {
    return WaterMaskOut(vec4<f32>(0.0,0.0,0.0,1.0),water_interface_depth(in.world_pos));
}

@fragment
fn fs_water_interface(in:WaterOut)->@builtin(frag_depth) f32 {
    return water_interface_depth(in.world_pos);
}

// Event history is small; all droplet/bubble trajectories are evaluated on GPU.
struct WaterParticleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) life: f32,
    @location(3) @interpolate(flat) kind: u32,
    @location(4) @interpolate(flat) level: f32,
    @location(5) normal: vec3<f32>,
};
fn particle_water_level(p:vec2<f32>,level:f32)->f32 {
    if(!water_fx_on(1u)){return level;}
    return level+water_swell(p,water_fx.clock.x,water_fx.counts.w).offset.z*smoothstep(0.08,2.4,level-water_fx_bed(p).x);
}
@vertex
fn vs_water_particle(@builtin(vertex_index) vertex: u32,@builtin(instance_index) instance: u32) -> WaterParticleOut {
    let counts=array<u32,4>(4u,6u,8u,12u); let count=counts[min(water_fx.counts.w,3u)]; let event=water_events[instance/count]; let index=instance%count;
    let age=water_fx.clock.x-event.position.w; let bubble=event.params.y==1.0;
    let entry=event.params.y==5.0;let fan=event.params.y==7.0;
    let footfall=fan && event.params.w>0.5;
    let h=water_fx_hash(vec2<f32>(f32(index)+event.motion.x*8.0,event.position.w*19.0+event.motion.y*8.0));
    let angle=h.x*6.283185; let direction=vec2<f32>(cos(angle),sin(angle));
    var pos=event.position.xyz; var size=0.004+h.y*0.006; var life=0.0;
    var velocity=vec3<f32>(0.0);var elongation=0.0;
    if(bubble && water_fx_on(128u)) {
        let rise=0.22+h.y*0.14;
        pos+=vec3<f32>(direction*0.035*sin(age*2.2+h.x)+event.motion.xy*age*0.025,rise*age);
        size*=1.0+age*0.1;
        life=smoothstep(0.0,0.15,age)*(1.0-smoothstep(2.5,3.0,age));
        life*=smoothstep(0.0,0.04,particle_water_level(pos.xy,event.params.z)-pos.z);
        let bubbles=2u+min(water_fx.counts.w,2u);
        if(index>=bubbles){life=0.0;}
    } else if ((event.params.y==0.0 || entry || fan) && water_fx_on(64u)) {
        var launch=0.85+h.y*0.65+event.motion.w*0.9;
        var horizontal=direction*(0.25+event.motion.w*0.5)+event.motion.xy*0.15;
        var origin=direction*event.params.x*0.5;
        var flight=age;
        if(entry) {
            // Each sector throws droplets through its own rim. Delayed release
            // breaks up the sheet instead of making a simultaneous particle ring.
            let theta=atan2(event.motion.y,event.motion.x)+(h.x-0.5)*event.params.w;
            let radial=vec2<f32>(cos(theta),sin(theta));
            launch=2.0+h.y*1.7+event.motion.w*1.1;
            horizontal=radial*(0.8+h.y*0.7+event.motion.w*0.65);
            origin=radial*event.params.x;
            flight=age-h.x*0.09;
            size=0.009+h.y*0.020;
        } else if(fan) {
            // Walking/strokes push water aside in low fans, rather than throwing
            // an isotropic fountain from the avatar's center.
            let forward=normalize(event.motion.xy+vec2<f32>(0.0001,0.0));
            let side=vec2<f32>(-forward.y,forward.x)*select(-1.0,1.0,index%2u==0u);
            let radial=normalize(forward*(0.15+h.x*0.45)+side);
            launch=0.6+h.y*0.5+event.motion.w*1.4;
            horizontal=radial*(0.45+event.motion.w*1.1)+event.motion.xy*0.06;
            origin=side*event.params.x*0.55+forward*0.12;
            size=0.007+h.y*0.008;
            if(footfall) {
                // A foot slap expels a small, uneven rim; most of the water
                // stays close to the shoe. Faster landings lift it farther.
                let power=sqrt(max(event.motion.w,0.0));
                let theta=6.283185*(f32(index)/f32(count)+h.x*0.12);
                let rim=vec2<f32>(cos(theta),sin(theta));
                launch=0.35+power*1.45+h.y*0.25;
                horizontal=rim*(0.25+power*0.85)+event.motion.xy*0.04;
                origin=rim*event.params.x*0.65;
                flight=age-h.x*0.035;
                size=0.004+h.y*0.006;
            }
        }
        let travel=(1.0-exp(-max(flight,0.0)*1.1))/1.1;
        pos+=vec3<f32>(origin+horizontal*travel,0.018+launch*flight-4.905*flight*flight);
        velocity=vec3<f32>(horizontal*exp(-max(flight,0.0)*1.1),launch-9.81*flight);
        elongation=min(length(velocity)*select(0.010,0.016,entry),select(0.035,0.09,entry));
        life=smoothstep(0.0,0.025,flight)*(1.0-smoothstep(select(0.28,0.7,entry),select(0.65,1.5,entry),flight));
        life*=smoothstep(-0.01,0.025,pos.z-particle_water_level(pos.xy,event.params.z));
        var droplets=select(min(count,max(3u,u32(ceil(event.motion.w*10.0)))),count,entry || fan);
        if(footfall){droplets=min(count,max(4u,2u+u32(ceil(event.motion.w*10.0))));}
        if(index>=droplets){life=0.0;}
    }
    if(age<0.0 || age>3.0){life=0.0;}
    let corners=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,1.0),vec2<f32>(-1.0,1.0));
    let corner=corners[vertex%6u];
    let segment=f32(vertex/6u);
    if(entry && index<2u) {
        // Two connected patches per sector leave droplets even at Low quality.
        // Both dives and cannonballs use the same geometry, scaled by impact.
        let radial=normalize(event.motion.xy+vec2<f32>(0.00001,0.0));
        let u=(f32(index)*3.0+segment+corner.x*0.5+0.5)/6.0;
        let top=corner.y*0.5+0.5;
        let theta=atan2(radial.y,radial.x)+(u-0.5)*event.params.w;
        let direction=vec2<f32>(cos(theta),sin(theta));
        let tip=0.9+0.075*sin(theta*7.0+event.position.w*3.0)+0.045*sin(theta*13.0-event.position.w);
        let height=max(0.0,(1.8+event.motion.w*1.05)*age-4.905*age*age)*tip;
        let radius=event.params.x+age*(0.85+event.motion.w*0.3)+top*top*height*0.5;
        pos=vec3<f32>(event.position.xy+direction*radius,event.params.z+0.012+top*height);
        life=smoothstep(0.0,0.035,age)*(1.0-smoothstep(0.65,1.2,age))*smoothstep(0.0,0.05,height);
        if(age<0.0 || !water_fx_on(64u)){life=0.0;}
        var crown:WaterParticleOut;
        crown.clip=frame.view_proj*vec4<f32>(pos,1.0);crown.world=pos;crown.uv=vec2<f32>(u*2.0-1.0,top*2.0-1.0);
        crown.life=life;crown.kind=2u;crown.level=event.params.z;
        crown.normal=normalize(vec3<f32>(direction,-top));
        return crown;
    }
    if(fan && index<2u) {
        let forward=normalize(event.motion.xy+vec2<f32>(0.0001,0.0));
        let side=vec2<f32>(-forward.y,forward.x)*select(-1.0,1.0,index==0u);
        var center=event.position.xy+forward*0.12+side*event.params.x*0.45;
        let top=corner.y*0.5+0.5;
        let across=(segment+corner.x*0.5+0.5)/3.0*2.0-1.0;
        let theta=atan2(side.y,side.x)+across*select(0.65,0.95,footfall);
        let radial=vec2<f32>(cos(theta),sin(theta));
        var height=max(0.0,(1.05+event.motion.w*1.5)*age-4.905*age*age);
        var radius=0.04+age*(0.5+event.motion.w)+top*height*0.65;
        if(footfall) {
            center=event.position.xy+forward*event.params.x*0.2+side*event.params.x*0.25;
            height=max(0.0,(0.35+sqrt(max(event.motion.w,0.0))*1.5)*age-4.905*age*age);
            radius=event.params.x*0.45+age*(0.25+event.motion.w*0.75)+top*height*0.4;
        }
        pos=vec3<f32>(center+radial*radius+event.motion.xy*min(age,0.3)*0.05,event.position.z+0.008+top*height);
        life=smoothstep(0.0,0.025,age)*smoothstep(0.0,select(0.035,0.012,footfall),height);
        if(age<0.0 || !water_fx_on(64u)){life=0.0;}
        var sheet:WaterParticleOut;
        sheet.clip=frame.view_proj*vec4<f32>(pos,1.0);sheet.world=pos;sheet.uv=vec2<f32>(across,corner.y);
        sheet.life=life;sheet.kind=4u;sheet.level=event.params.z;
        sheet.normal=normalize(vec3<f32>(radial,-top*1.3));
        return sheet;
    }
    if(vertex>=6u){size=0.0;elongation=0.0;}
    let projected=vec2<f32>(dot(velocity,frame.cam_right.xyz),dot(velocity,frame.cam_up.xyz));
    let along=normalize(projected+vec2<f32>(0.00001,0.001));
    let across=vec2<f32>(along.y,-along.x);
    let offset=across*corner.x*size+along*corner.y*(size+elongation);
    pos+=frame.cam_right.xyz*offset.x+frame.cam_up.xyz*offset.y;
    var out: WaterParticleOut; out.clip=frame.view_proj*vec4<f32>(pos,1.0);out.world=pos;out.uv=corner;
    out.life=life;out.kind=select(select(0u,3u,entry),1u,bubble);out.level=event.params.z;out.normal=vec3<f32>(0.0,0.0,1.0);
    return out;
}
fn water_particle_color(in: WaterParticleOut) -> vec4<f32> {
    if(in.kind==2u || in.kind==4u) {
        let top=in.uv.y*0.5+0.5;
        let ragged=water_fx_noise(in.world.xy*7.0+vec2<f32>(in.level,top*3.0));
        let fine=water_fx_noise(in.world.xy*19.0+top*2.0);
        let holes=smoothstep(0.14+top*0.43,0.30+top*0.43,ragged*0.7+fine*0.3);
        let edge=smoothstep(0.72,0.98,top)*(0.55+fine*0.45);
        let view=normalize(frame.camera_pos.xyz-in.world);
        let normal=normalize(in.normal+vec3<f32>((ragged-0.5)*0.12,(fine-0.5)*0.12,0.0));
        let fresnel=0.02037+0.97963*pow(1.0-abs(dot(normal,view)),5.0);
        var alpha=(0.22+fresnel*0.28+edge*0.32)*holes*in.life;
        if(in.life<=0.001 || alpha<0.005){discard;}
        let uv=clamp(screen_uv(in.clip.xy)+vec2<f32>(dot(normal,frame.cam_right.xyz),-dot(normal,frame.cam_up.xyz))*0.006*in.life,vec2<f32>(0.001),vec2<f32>(0.999));
        let transmitted=textureSampleLevel(scene_tex,lin_clamp,uv,0.0).rgb;
        let reflection=reflect(-view,normal);
        let reflected=sky_color(vec3<f32>(reflection.xy,abs(reflection.z)));
        let light=frame.sky_horizon.rgb*0.4+frame.sun_color.rgb*max(frame.sun_dir.z,0.0)*0.4;
        let color=mix(transmitted*0.94,reflected,fresnel*0.7)+light*(0.04+edge*select(0.42,0.65,in.kind==4u));
        let depth=scene_depth_at(screen_uv(in.clip.xy));
        if(depth>0.0) {
            let behind=world_from_depth(screen_uv(in.clip.xy),depth);
            alpha*=clamp((distance(behind,frame.camera_pos.xyz)-distance(in.world,frame.camera_pos.xyz))/0.12,0.0,1.0);
        }
        return vec4<f32>(apply_fog(color,in.world)*alpha,alpha);
    }
    let radius=length(in.uv); if(radius>1.0 || in.life<=0.001){discard;}
    var alpha=(1.0-smoothstep(0.2,1.0,radius))*0.4;
    if(in.kind==1u){alpha=exp(-pow((radius-0.8)*14.0,2.0))*0.3+exp(-length(in.uv-vec2<f32>(-0.25,0.3))*18.0)*0.1;}
    let depth=scene_depth_at(screen_uv(in.clip.xy));
    if(depth>0.0){let behind=world_from_depth(screen_uv(in.clip.xy),depth);alpha*=clamp((distance(behind,frame.camera_pos.xyz)-distance(in.world,frame.camera_pos.xyz))/0.15,0.0,1.0);}
    let sphere=normalize(vec3<f32>(in.uv,sqrt(max(1.0-radius*radius,0.001))));
    let normal=frame.cam_right.xyz*sphere.x+frame.cam_up.xyz*sphere.y+normalize(frame.camera_pos.xyz-in.world)*sphere.z;
    let reflected=sky_color(reflect(normalize(in.world-frame.camera_pos.xyz),normal));
    let sun=pow(max(dot(reflect(-normalize(frame.sun_dir.xyz),normal),normalize(frame.camera_pos.xyz-in.world)),0.0),64.0);
    var lighting=reflected*0.35+frame.sun_color.rgb*sun*0.5+frame.sky_horizon.rgb*0.15;
    if(in.kind==3u){lighting+=frame.sky_horizon.rgb*0.32+frame.sun_color.rgb*0.12;alpha*=1.3;}
    alpha*=in.life;
    return vec4<f32>(apply_fog(lighting,in.world)*alpha,alpha);
}
@fragment
fn fs_water_particle(in: WaterParticleOut) -> @location(0) vec4<f32> {
    if(!camera_underwater() && in.world.z<in.level){discard;}
    return water_particle_color(in);
}
@fragment
fn fs_water_particle_under(in: WaterParticleOut) -> @location(0) vec4<f32> {
    if(camera_underwater() || in.world.z>=in.level){discard;}
    return water_particle_color(in);
}
