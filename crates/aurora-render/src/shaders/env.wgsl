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
const STAR_MARGIN: f32 = 0.025;     // stars.rs MARGIN (radians)
// a cell holds about 6 stars, up to a few dozen in the Milky Way
const STAR_MAX_PER_CELL: u32 = 96u;
// the Milky Way's light at its brightest, and the size of its points
// (sigma, pixels)
const MW_GLOW: f32 = 0.12;
const MW_GRAIN_SIGMA: f32 = 0.6;
// the band's light left as a haze, the rest in points; points per lattice
// cell at the band's brightest; the floor of the points' mean (see milky_way)
const MW_HAZE: f32 = 0.45;
const MW_POINTS: f32 = 2.0;
const MW_POINT_MEAN_MIN: f32 = 0.6;

struct GpuStar {
    pos: vec4<f32>,   // unit direction, peak intensity
    color: vec4<f32>, // linear colour, point size (pixels)
};

// The Milky Way map (stars.rs MILKY_WAY_*): galactic longitude -180° to
// 180° across, latitude -MW_REACH to +MW_REACH down.
const MW_WIDTH: u32 = 2048u;
const MW_HEIGHT: u32 = 512u;
const MW_REACH: f32 = 0.785398163; // 45°

struct StarSky {
    cells: array<u32, 6920>, // 2 * STAR_CELL_TABLE: real sky, then the Legacy Viewer's
    stars: array<GpuStar>,
};

@group(0) @binding(24) var<storage, read> star_sky: StarSky;
// The Milky Way map (stars.rs milky_way_texels), MW_WIDTH x MW_HEIGHT, one
// packed texel per element (mw_texel)
@group(0) @binding(25) var milky_way_map: texture_2d<u32>;

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

// Pixel size of this pass: `frame.sky_star_pass` when set (the water and
// mirror reflections draw into smaller targets than `frame.screen`), else
// `frame.screen`.
fn sky_pass_pixel() -> vec2<f32> {
    if (frame.sky_star_pass.x > 0.0) {
        return frame.sky_star_pass.zw;
    }
    return frame.screen.zw;
}

fn sky_ray(ndc: vec2<f32>) -> vec3<f32> {
    let w = frame.inv_view_proj_rot * vec4<f32>(ndc, 0.5, 1.0);
    return normalize(w.xyz / w.w);
}

// How the region direction changes over one pixel of this pass, across and
// down, at `ndc`: the pixel's own footprint, so stars keep a round shape of
// the same size in pixels anywhere in the view and in off-centre photo
// tiles.
struct SkyFootprint {
    across: vec3<f32>,
    down: vec3<f32>,
};

fn sky_footprint(ndc: vec2<f32>, dir: vec3<f32>) -> SkyFootprint {
    let step = 2.0 * sky_pass_pixel();
    var f: SkyFootprint;
    f.across = sky_ray(ndc + vec2<f32>(step.x, 0.0)) - dir;
    f.down = sky_ray(ndc + vec2<f32>(0.0, step.y)) - dir;
    return f;
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

// One texel of the Milky Way map: glow (square root of the light), grain
// (stars of V 6.5 to 9.5 per texel, x 16) and B-V colour (x 100), 0..255.
fn mw_texel(x: i32, y: i32) -> vec3<f32> {
    let xi = (x + i32(MW_WIDTH)) % i32(MW_WIDTH);
    let yi = clamp(y, 0, i32(MW_HEIGHT) - 1);
    let t = textureLoad(milky_way_map, vec2<i32>(xi, yi), 0).x;
    return vec3<f32>(f32(t & 255u), f32((t >> 8u) & 255u), f32((t >> 16u) & 255u));
}

// The map between texel centres (`uv` in texels, centres on the integers).
fn mw_bilinear(uv: vec2<f32>) -> vec3<f32> {
    let i = vec2<i32>(floor(uv));
    let f = uv - floor(uv);
    return mix(mix(mw_texel(i.x, i.y), mw_texel(i.x + 1, i.y), f.x), mix(mw_texel(i.x, i.y + 1), mw_texel(i.x + 1, i.y + 1), f.x), f.y);
}

// The chance that a lattice cell holds a point, from the map's light and its
// counted stars there.
fn mw_occupancy(t: vec3<f32>) -> f32 {
    let glow = (t.x / 255.0) * (t.x / 255.0);
    return 1.0 - exp(-(glow * MW_POINTS + t.y / 16.0 / 4.0));
}

// The Milky Way at catalogue direction `q` (J2000 equatorial), linear,
// from the map of the real stars too faint for the naked eye (stars.rs,
// Tycho-2): their glow, with its clouds and dark lanes, and their grain,
// faint points scattered as densely as the counted stars, fixed in the
// sky. `ax`, `ay`: the pixel's footprint across and down (catalogue
// frame), `pixel` its angle (radians).
fn milky_way(q: vec3<f32>, ax: vec3<f32>, ay: vec3<f32>, pixel: f32) -> vec3<f32> {
    // J2000 equatorial to galactic (IAU 1958, Hipparcos values; the same
    // rows as scripts/generate-star-catalog.py)
    let m0 = vec3<f32>(-0.0548756, -0.8734371, -0.4838350);
    let m1 = vec3<f32>(0.4941094, -0.4448296, 0.7469822);
    let m2 = vec3<f32>(-0.8676661, -0.1980764, 0.4559838);
    let g = vec3<f32>(dot(m0, q), dot(m1, q), dot(m2, q));
    let b = asin(clamp(g.z, -1.0, 1.0));
    if (abs(b) >= MW_REACH) {
        return vec3<f32>(0.0);
    }
    let l = atan2(g.y, g.x);
    // texel coordinates, centres on the integers
    let uv = vec2<f32>((l / 6.28318531 + 0.5) * f32(MW_WIDTH), (b / MW_REACH * 0.5 + 0.5) * f32(MW_HEIGHT)) - 0.5;
    let t = mw_bilinear(uv);
    if (t.x + t.y < 0.5) {
        return vec3<f32>(0.0);
    }
    let glow = (t.x / 255.0) * (t.x / 255.0);
    // dust reddens the light towards the centre; the eye sees little colour
    let warm = clamp((t.z / 100.0 - 0.45) / 0.9, 0.0, 1.0);
    let tint = mix(vec3<f32>(0.82, 0.88, 1.0), vec3<f32>(1.0, 0.87, 0.70), warm);
    let light = tint * glow * MW_GLOW;

    // The band is made of stars: part of its light stays a haze (the stars
    // too faint to tell apart), the rest is gathered into faint points on a
    // lattice fixed in the sky, two cells per texel each way, each holding a
    // star or none, as many as the map's light and its counted stars say at
    // the cell's centre (so every pixel that sees a point agrees whether it
    // is there). The points keep the band's mean light; where a pixel covers
    // more than a cell (wide views, reflection probes) they would shimmer,
    // and give way to that mean.
    let cell = MW_REACH * 2.0 / f32(MW_HEIGHT * 2u);
    let sparkle = 1.0 - smoothstep(0.8 * cell, 1.6 * cell, pixel);
    if (sparkle <= 0.0) {
        return light;
    }
    // the chance here, for the field's expected mean below
    let p = mw_occupancy(t);
    // the pixel's footprint along longitude (east) and latitude (north),
    // and back: a point's offset in pixels
    let east = vec3<f32>(-sin(l), cos(l), 0.0);
    let north = vec3<f32>(-sin(b) * cos(l), -sin(b) * sin(l), cos(b));
    let gx = vec3<f32>(dot(m0, ax), dot(m1, ax), dot(m2, ax));
    let gy = vec3<f32>(dot(m0, ay), dot(m1, ay), dot(m2, ay));
    let j = mat2x2<f32>(vec2<f32>(dot(east, gx), dot(north, gx)), vec2<f32>(dot(east, gy), dot(north, gy)));
    let det = j[0].x * j[1].y - j[1].x * j[0].y;
    if (abs(det) < 1e-20) {
        return light;
    }
    let inv = mat2x2<f32>(vec2<f32>(j[1].y, -j[0].y), vec2<f32>(-j[1].x, j[0].x)) * (1.0 / det);
    let c = uv * 2.0 + 0.5; // cell coordinates
    // the pixel's cell and its eight neighbours: a point of any other cell
    // is at least a cell away, where each point has faded to nothing
    // (`reach`), so none is cut off as the sky turns
    let base = vec2<i32>(floor(c)) - 1;
    let reach = cell * max(cos(b), 0.1) / max(pixel, 1e-9);
    var field = vec3<f32>(0.0);
    for (var k = 0; k < 9; k++) {
        let ci = base + vec2<i32>(k % 3, k / 3);
        let h = pcg3(vec3<u32>(u32((ci.x + i32(MW_WIDTH * 2u)) % i32(MW_WIDTH * 2u)), u32(ci.y + 4), 0x5717u));
        let r = vec3<f32>(h & vec3<u32>(0xffffu)) / 65535.0;
        // the cell's centre is at c = ci + 0.5, so uv = ci / 2
        if (r.z >= mw_occupancy(mw_bilinear(vec2<f32>(ci) * 0.5))) {
            continue;
        }
        let d = (vec2<f32>(ci) + r.xy - c) * vec2<f32>(cell * cos(b), cell);
        let px = inv * d;
        let r2 = dot(px, px) / (MW_GRAIN_SIGMA * MW_GRAIN_SIGMA);
        let fade = 1.0 - smoothstep(0.5 * reach, reach, sqrt(dot(px, px)));
        if (r2 < 9.0 && fade > 0.0) {
            // brightness: the faint ones the more numerous (star counts
            // grow about 2.5 times a magnitude over three magnitudes), mean 1
            let u = mix(0.0316, 1.0, f32(h.z >> 16u) / 65535.0);
            let w = 0.518 * pow(u, -0.6);
            // a little colour of its own, bluer or redder than the band
            let hue = f32((h.x >> 16u) & 255u) / 255.0 - 0.5;
            field += w * fade * exp(-0.5 * r2) * (vec3<f32>(1.0) + vec3<f32>(0.12, 0.0, -0.12) * hue);
        }
    }
    // the field's mean over a pixel: occupied cells per pixel times a
    // point's area (within its fade, about). The points are scaled by it, but
    // never by less than a floor, so that zooming in shows separate points
    // rather than ever brighter ones; the light the floor holds back stays as
    // haze, so the band keeps its mean light at every zoom.
    let cells = pixel * pixel / (cell * cell * max(cos(b), 0.1));
    let kept = 0.75 * reach / MW_GRAIN_SIGMA;
    let area = 6.2831853 * MW_GRAIN_SIGMA * MW_GRAIN_SIGMA * (1.0 - exp(-0.5 * kept * kept));
    let expected = p * area * cells;
    let mean = max(expected, MW_POINT_MEAN_MIN);
    let points = mix(vec3<f32>(1.0), field / mean + vec3<f32>(1.0 - expected / mean), sparkle);
    return light * (MW_HAZE + (1.0 - MW_HAZE) * points);
}

// Stars and Milky Way in region direction `d` (unit), linear, before the
// clouds' cover. `background` is the sky's own linear colour there: in the
// real sky a bright sky drowns them (twilight shows the brightest first,
// daylight none), where the Legacy Viewer adds its stars whatever the sky.
// LLDrawPoolWLSky::renderStarsDeferred passes the sky's star_brightness / 500, which starsF.glsl turns into
// smoothstep(0, 0.9, ...) * 32 on the star texture's alpha: full brightness
// from about 100, dimmer only below. The same curve, without the 32 times
// overdrive.
fn sky_stars(d: vec3<f32>, background: vec3<f32>, foot: SkyFootprint) -> vec3<f32> {
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
    // the pixel's footprint in the catalogue's frame
    let ax = vec3<f32>(dot(frame.sky_stars[0].xyz, foot.across), dot(frame.sky_stars[1].xyz, foot.across), dot(frame.sky_stars[2].xyz, foot.across));
    let ay = vec3<f32>(dot(frame.sky_stars[0].xyz, foot.down), dot(frame.sky_stars[1].xyz, foot.down), dot(frame.sky_stars[2].xyz, foot.down));
    let ax2 = max(dot(ax, ax), 1e-14);
    let ay2 = max(dot(ay, ay), 1e-14);
    let pixel = sqrt(max(ax2, ay2));
    let t = frame.camera_pos.w;
    let cell = select(0u, STAR_CELL_TABLE, legacy) + star_cell(q);
    let first = star_sky.cells[cell];
    let last = min(star_sky.cells[cell + 1u], first + STAR_MAX_PER_CELL);
    var sum = vec3<f32>(0.0);
    for (var i = first; i < last; i++) {
        let s = star_sky.stars[i];
        // the star's offset from this pixel, in this pass's pixels
        let off = s.pos.xyz - q;
        let px = dot(off, ax) / ax2;
        let py = dot(off, ay) / ay2;
        // the real stars' size is in pixels; the Legacy Viewer's is an
        // angle in the sky (stars.rs legacy_sigma), at least 0.75 px wide
        let sigma = select(s.color.w, max(s.color.w / pixel, 0.75), legacy);
        let r2 = (px * px + py * py) / (sigma * sigma);
        // stars are listed in the cells within STAR_MARGIN only: on coarse
        // targets (reflection probes) the widest points are cut there
        let reach = STAR_MARGIN / (pixel * sigma);
        let reach2 = reach * reach;
        // the bright stars' soft glow, four times wider (the eye's
        // scattering, as on a photograph of the sky)
        if (s.pos.w > 0.3 && r2 < min(144.0, reach2)) {
            sum += s.color.rgb * (s.pos.w - 0.3) * 0.05 * exp(-0.5 * r2 / 16.0);
        }
        if (r2 < min(9.0, reach2)) {
            // scintillation: slow and slight overhead, stronger low down
            // (the Legacy Viewer's stars flicker, starsF.glsl twinkle)
            let seed = hash3(floor(s.pos.xyz * 4096.0));
            let depth = select(mix(0.35, 0.05, clamp(d.z * 2.0, 0.0, 1.0)), 0.4, legacy);
            let wave = sin(t * (2.3 + 3.1 * seed) + seed * 40.0) * sin(t * (0.9 + 1.3 * seed) + seed * 17.0);
            sum += s.color.rgb * s.pos.w * exp(-0.5 * r2) * (1.0 + depth * wave);
        }
    }
    // Preferences › "Star brightness" (1 for the Legacy Viewer's stars)
    var col = sum * max(frame.sky_stars[2].w, 0.0);
    if (frame.sky_stars[1].w > 0.0) {
        col += milky_way(q, ax, ay, pixel) * frame.sky_stars[1].w;
    }
    return col * gain * extinction * glare;
}

// Texel of a heavenly body's texture in direction `dir`, as stored: rgb in the
// texture's sRGB, not premultiplied, a its alpha (0 outside the quad).
// `radius` is LLHeavenBody's disc radius (SUN_DISK_RADIUS 0.5, the moon 0.9
// of it). LLVOSky::updateHeavenlyBodyGeometry lays the quad at
// HEAVENLY_BODY_DIST with half sides HEAVENLY_BODY_DIST x 0.1 x radius x
// scale, wider and taller towards the horizon: as tangents, 0.1 x radius x
// scale x (1 + 0.3 (1 - z)) across and (1 + 0.2 (1 - z)) up.
fn body_disc(dir: vec3<f32>, body: vec3<f32>, radius: f32, scale: f32, slot: u32) -> vec4<f32> {
    let b = normalize(body);
    let low = 1.0 - b.z;
    let half_size = vec2<f32>(1.0 + low * 0.3, 1.0 + low * 0.2) * (0.1 * radius * scale);
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
    return sample_tex_level(slot, uv, 0.0);
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

// The sky seen along a ray whose own footprint is not known (bent by the
// water, reflected): the size of the view's middle pixel, laid at right
// angles to the ray. The middle pixel's own axes would lie along a ray far
// from the view and draw its stars as dashes.
fn sky_full(dir: vec3<f32>) -> vec3<f32> {
    let centre = sky_ray(vec2<f32>(0.0));
    let middle = sky_footprint(vec2<f32>(0.0), centre);
    let d = normalize(dir);
    let side = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.z) > 0.9);
    let u = normalize(cross(side, d));
    var foot: SkyFootprint;
    foot.across = u * length(middle.across);
    foot.down = cross(d, u) * length(middle.down);
    return sky_full_at(dir, foot);
}

fn sky_full_at(dir: vec3<f32>, foot: SkyFootprint) -> vec3<f32> {
    let d = normalize(dir);
    if (frame.studio_aurora[0].w > 0.0) {
        let c = studio_sky_color(d);
        return c + sky_stars(d, c, foot) * smoothstep(0.01, 0.08, d.z);
    }
    let a = atmos_eval(d, false);
    // skyF.glsl: added to the haze color before its x 2, under the clouds
    let haze = a.color + sky_rainbow(a.light_dot) + sky_halo(a.light_dot);
    var c = clamp(haze * 2.0, vec3<f32>(0.0), vec3<f32>(5.0));
    // Sun then moon over the haze, under the clouds, like
    // LLDrawPoolWLSky::renderHeavenlyBodies (lldrawpoolwlsky.cpp): plain alpha
    // blending (BT_ALPHA) of the texel as stored, in the same space as the
    // haze, so softenLightF's srgb_to_linear x sky_hdr_scale below applies to
    // both. sunDiscF.glsl draws the texel as it is; moonF.glsl scales its rgb
    // by the moon brightness, fades its alpha near the horizon and discards
    // alpha <= 2/255. Premultiplying in linear and adding the sun x 6 (moon at
    // 90 % opacity) blew soft halos out to a white disc with stepped rings and
    // a visible quad (#2782).
    var hidden = 0.0;
    let sd = normalize(frame.sky_sun.xyz);
    if (frame.tex_slots.y != 0u) {
        let s = body_disc(d, sd, 0.5, frame.sky_sun.w, frame.tex_slots.y);
        c = mix(c, s.rgb, s.a);
        // the Legacy Viewer's sun quad writes depth and hides every star
        // behind it, even where it is clear: here only as much as it covers
        hidden = s.a;
    }
    if (frame.tex_slots.z != 0u) {
        let md = normalize(frame.sky_moon.xyz);
        let m = body_disc(d, md, 0.45, frame.sky_moon.w, frame.tex_slots.z);
        if (m.a > 2.0 / 255.0) {
            let fade = select(1.0, clamp(md.z * md.z * 4.0, 0.0, 1.0), md.z > 0.0);
            c = mix(c, m.rgb * frame.sky_misc.z, m.a * fade);
            // SL-14113: the moon's kept texels write depth, the stars behind
            // them are hidden
            hidden = 1.0;
        }
    }
    let clouds = sky_clouds(d);
    c = mix(c, clouds.rgb, clouds.a);
    var col = srgb_to_linear(c) * frame.sky_cloud_pd2.w;
    let cover = 1.0 - clouds.a;
    // stars and Milky Way (above) behind the clouds and the sun and moon; the
    // aurora, in the atmosphere, behind the clouds only
    col += (sky_stars(d, col, foot) * (1.0 - hidden) + aurora_sky(d)) * cover;
    if (frame.tex_slots.y == 0u) {
        let mu = dot(d, sd);
        let disk = smoothstep(0.99955, 0.9997, mu) * clamp(sd.z * 10.0 + 1.0, 0.0, 1.0);
        col += frame.sun_color.rgb * disk * 6.0 * cover;
    }
    return col;
}

fn lattice_gradient(i: vec3<f32>) -> vec3<f32> {
    let h = pcg3(vec3<u32>(vec3<i32>(i) + vec3<i32>(4096)));
    return normalize(vec3<f32>(h & vec3<u32>(0xffffu)) / 32767.5 - 1.0 + vec3<f32>(1e-4));
}

fn corner(i: vec3<f32>, f: vec3<f32>, o: vec3<f32>) -> f32 {
    return dot(lattice_gradient(i + o), f - o);
}

// Gradient noise in 0..1, quintic fade: no facets along the lattice. The
// northern lights' folds and rays (the Milky Way, which it was written for,
// is a texture now).
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

// Aurora Weather: the aurora (northern lights), at night. Curtains of light
// stand in a layer high over a flat sky, from their foot (height 1) to their
// top (3.5), along a line that folds and drifts, `dist` away toward their
// heading so that their foot is seen at the height chosen (5° to 80° up).
// The view is marched through the stretch of the layer where it can meet
// the curtain (16 steps at High, fewer at a lower Weather quality,
// jittered): each step adds the sheet it passes, thin, streaked with rays
// that flicker; green at the foot, violet or red toward the top
// (frame.aurora.w). Seen through more of the layer near the horizon, it
// brightens there, as a real one does.
fn aurora_sky(d: vec3<f32>) -> vec3<f32> {
    let a = frame.aurora;
    if (a.x <= 0.001 || d.z <= 0.005) { return vec3<f32>(0.0); }
    // night only: from the sun a little under the horizon
    let night = 1.0 - smoothstep(-0.2, 0.0, normalize(frame.sky_sun.xyz).z);
    if (night <= 0.0) { return vec3<f32>(0.0); }
    let t = frame.camera_pos.w;
    let toward = vec2<f32>(sin(a.y), cos(a.y));
    let side = vec2<f32>(toward.y, -toward.x);
    let elev = mix(0.09, 1.4, clamp(a.z, 0.0, 1.0));
    let dist = 1.0 / tan(elev);
    let green = vec3<f32>(0.12, 1.0, 0.38);
    var top = green * vec3<f32>(0.7, 0.8, 1.2);
    if (a.w > 1.5) {
        top = vec3<f32>(1.0, 0.12, 0.22);
    } else if (a.w > 0.5) {
        top = vec3<f32>(0.55, 0.2, 1.0);
    }
    // the view meets the curtain's line where it is `dist` (give or take
    // its folds) toward the heading: the march spans only that stretch
    let k = dot(d.xy, toward) / d.z;
    if (k <= 0.02) { return vec3<f32>(0.0); }
    let swing = 0.4 + 0.15 * dist;
    let reach = 1.6 * swing + 0.3;
    let h0 = max(1.0, (dist - reach) / k);
    let h1 = min(3.5, (dist + reach) / k);
    if (h1 <= h0) { return vec3<f32>(0.0); }
    // 16 steps at High, 10 at Medium, 6 at Low (the Weather quality)
    let steps = select(select(16, 10, rain_quality() == 1), 6, rain_quality() <= 0);
    let dh = (h1 - h0) / f32(steps);
    let jitter = fract(sin(dot(d.xy * 400.0, vec2<f32>(12.9898, 78.233))) * 43758.5453);
    var col = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let h = h0 + (f32(i) + jitter) * dh;
        let p = d.xy * (h / d.z);
        let u = dot(p, side);
        let v = dot(p, toward);
        // the curtain's line: broad folds and smaller kinks, drifting
        let fold = (gradient_noise(vec3<f32>(u * 0.3, 0.0, t * 0.015)) - 0.5) * 2.4
            + (gradient_noise(vec3<f32>(u * 1.2, 4.0, t * 0.04)) - 0.5) * 0.7;
        let across = v - dist - fold * swing;
        let width = 0.06 + 0.03 * h;
        if (abs(across) > width * 3.5) { continue; }
        let sheet = exp(-across * across / (width * width));
        // its rays: streaks up the curtain, flickering
        let rays = 0.3 + 0.7 * gradient_noise(vec3<f32>(u * 7.0, 9.0, t * 0.3));
        // a sharp foot, fading toward the top
        let rise = smoothstep(1.0, 1.12, h) * exp(-(h - 1.0) * 0.8);
        // so far along it, around its heading
        let span = exp(-u * u / (dist * dist * 6.0 + 16.0));
        col += sheet * rays * rise * span * dh * mix(green, top, smoothstep(1.4, 3.0, h));
    }
    // seen through more of the layer the lower the view (to a limit)
    let path = min(1.0 / d.z, 8.0);
    let horizon = smoothstep(0.005, 0.06, d.z);
    return col * path * a.x * night * horizon * 1.5;
}

// The lightning lights the clouds where it struck: brightest toward it, at
// the height of the cloud base seen from here (low for a far strike, high
// for a near one), spreading over the sky, a faint flash all round.
fn lightning_glow(dir: vec3<f32>) -> vec3<f32> {
    let l = frame.lightning;
    if (l.w <= 0.001) { return vec3<f32>(0.0); }
    let flat_len = length(dir.xy);
    let toward = select(0.0, max(dot(dir.xy / max(flat_len, 1e-4), l.xy), 0.0), flat_len > 1e-4);
    let around = pow(toward, 3.0);
    // (squared by hand: pow of a negative base is undefined in SPIR-V and MSL)
    let dz = (dir.z - l.z) / 0.3;
    let height = exp(-dz * dz);
    let above = smoothstep(-0.05, 0.08, dir.z);
    return vec3<f32>(0.78, 0.82, 1.0) * l.w * above * (around * height * 1.6 + 0.15);
}

fn sky_pixel(in: SkyOut) -> vec4<f32> {
    // from the camera, not region positions minus the camera: those rounded
    // differently each frame and the sky shook as it moved (#1059)
    let dir = sky_ray(in.ndc);
    // Under the rim of the dome a probe's capture has no sky: LLVOWLSky::calcPhi
    // ends the dome at pi/8 from its apex, and LLDrawPoolWLSky::renderDome
    // lowers it by the dome offset (about 5.4 degrees under the horizon by
    // default). The Legacy Viewer's captures draw the water there; Aurora's have
    // no water pass, so this stands in for the open sea around the terrain: the
    // sky in the mirror, by a Fresnel term. Haze here lit the sides of everything
    // outdoors; black left a band the Legacy Viewer does not have.
    if ((frame.light_mode.z & PROBE_CAPTURE_DOME) != 0u) {
        let rim_up = 0.92387953 - frame.sky_misc.x;
        let d = normalize(dir);
        if (d.z < rim_up / sqrt(0.14644661 + rim_up * rim_up)) {
            let fresnel = 0.02 + 0.98 * pow(1.0 - min(abs(d.z), 1.0), 5.0);
            return vec4<f32>(sky_color(vec3<f32>(d.x, d.y, abs(d.z))) * fresnel, 1.0);
        }
    }
    var col = sky_full_at(dir, sky_footprint(in.ndc, dir));
    col += lightning_glow(dir);
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
    out.clip = eye_clip(rec, ep);
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
    out.clip = eye_clip(rec, ep);
    out.world_pos = ep.xyz + frame.eye.xyz;
    out.normal = vertex_normal(rec, in);
    let raw = vec4<f32>(terrain_tangent(in.weights), in.normal.w);
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
    let ramp_loc = tex_loc.values[6u];
    let uv = vec2<f32>(c, 1.0 - noise);
    let gx = dx * vec2<f32>(1.0, -1.0);
    let gy = dy * vec2<f32>(1.0, -1.0);
    let a = textureSampleGrad(tex_pages[texture_page_index(ramp_loc)], clamp_sampler, uv, ramp_loc & 0xffffu, gx, gy).r;
    let b = textureSampleGrad(tex_pages[texture_page_index(ramp_loc)], clamp_sampler, uv - vec2<f32>(2.0, 0.0), ramp_loc & 0xffffu, gx, gy).r;
    let f = textureSampleGrad(tex_pages[texture_page_index(ramp_loc)], clamp_sampler, uv - vec2<f32>(1.0, 0.0), ramp_loc & 0xffffu, gx, gy).r;
    return vec4<f32>(f * a, f * (1.0 - a), (1.0 - f) * b, (1.0 - f) * (1.0 - b));
}

fn terrain_mix_weights(in: TerrainPbrOut, dx: vec3<f32>, dy: vec3<f32>, blend_dx: vec2<f32>, blend_dy: vec2<f32>, clamp_sampler: sampler) -> vec4<f32> {
    let rec = records[in.record];
    if (rec.flags.z != 0u) {
        // LLTerrainPaintMap stores slots 2..4 in linear RGB; slot 1 is the
        // remainder. No height/noise threshold is applied to a paintmap.
        let slot = rec.flags.z;
        let loc = tex_loc.values[slot];
        // Clamp each mip level at region edges; detail maps use repeat.
        let uv = in.region_pos.xy / rec.mat_uv.xy;
        var other = max(textureSampleGrad(tex_pages[texture_page_index(loc)], clamp_sampler, uv, loc & 0xffffu, dx.xy / rec.mat_uv.xy, dy.xy / rec.mat_uv.xy).rgb, vec3<f32>(0.0));
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
    let loc = tex_loc.values[slot];
    return textureSampleGrad(tex_pages[texture_page_index(loc)], tex_sampler, uv, loc & 0xffffu, dx, dy);
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
        wet_light=(sky_color(reflect(-v,n))*fresnel*0.25+frame.sun_color.rgb*min(glint,2.0)*select(sh, shadow_lift(sh), is_classic_sky()))*wet;
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
    // the terrain stays put: the camera's motion alone
    let ep = vec4<f32>(in.world_pos - frame.eye.xyz, 1.0);
    return depth_normal(in.normal, front, frame.motion_vp * ep, frame.motion_prev_vp * ep, 0.0);
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

// A unit vector, or straight up when `v` has no length: a normal scale of
// 0 (a valid EEP value) leaves the waves flat, and normalize(0) gives NaN,
// which turned the whole surface white or black.
fn water_unit(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if (!(l > 1e-6)) { return vec3<f32>(0.0, 0.0, 1.0); }
    return v / l;
}

// pbrPunctual (deferredUtil.glsl) for the water: metallic 1, so no diffuse;
// returns clamp(nl x specContrib, 0, 10), before the light, shadow and haze.
fn legacy_water_punctual(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32, specular: vec3<f32>) -> vec3<f32> {
    let perceptual = max(roughness, 8.0 / 255.0);
    let a = perceptual * perceptual;
    let reflectance90 = clamp(max(max(specular.r, specular.g), specular.b) * 25.0, 0.0, 1.0);
    let h = normalize(l + v);
    let nl = clamp(dot(n, l), 0.001, 1.0);
    let nv = clamp(abs(dot(n, v)), 0.001, 1.0);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let f = specular + (vec3<f32>(reflectance90) - specular) * pow(1.0 - vh, 5.0);
    let a2 = a * a;
    let g = 2.0 * nl / (nl + sqrt(a2 + (1.0 - a2) * nl * nl)) * (2.0 * nv / (nv + sqrt(a2 + (1.0 - a2) * nv * nv)));
    let dd = (nh * a2 - nh) * nh + 1.0;
    let d = a2 / (PI * dd * dd);
    return clamp(nl * (f * g * d / (4.0 * nl * nv)), vec3<f32>(0.0), vec3<f32>(10.0));
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
    var wavef = (wave1 + wave2 * 0.4 + wave3 * 0.6) * 0.5;
    // rain prototype: the puddles' rings in the waves themselves, so they
    // bend the reflection and the refraction and ride the waves
    let rain_footprint = max(length(dpdx(in.world_pos.xy)), length(dpdy(in.world_pos.xy)));
    wavef = vec3<f32>(wavef.xy + rain_water_rings(in.world_pos, rain_footprint), wavef.z);
    var n = wavef * frame.water_normal.xyz;
    n.z *= 2.0;
    let footprint=max(length(dpdx(in.world_pos.xy)),length(dpdy(in.world_pos.xy)));
    let wake=water_wake_sample(in.world_pos.xy,in.level);
    let furrow=1.0-smoothstep(0.003,0.035,-wake.z)*0.7;
    if(water_fx_on(1u)) {
        let micro=water_micro(in.world_pos.xy,frame.camera_pos.w,water_fx.counts.w,footprint);
        let detail=clamp(length(frame.water_normal.xy)*0.35,0.5,1.5);
        n=normalize(vec3<f32>((n.xy*0.18-micro.xy*detail)*furrow,1.0)+water_fx_normal(in.world_pos.xy,in.level,wake.xy));
    } else { n=water_unit(n+water_fx_normal(in.world_pos.xy,in.level,wake.xy)); }
    // waterF.glsl: the reflection follows the waves scaled by the EEP normal
    // scale (wave_ibl), the sun glint the unscaled waves flattened with
    // distance (enhanced water off). With a normal scale of 0, wave_ibl is
    // normalize(0), NaN on the GPU, and what follows depends on the driver:
    // NaN through mix/max, or a probe lookup giving 0. We take the latter:
    // the water keeps its refraction and fog and loses its reflection,
    // which is what the reported region shows in the Legacy Viewer (a flat,
    // dark sea) and what its creator chose. That rule holds with enhanced
    // water on too: its own normals would otherwise mirror the sky.
    // legacy_water: no enhanced effect at all (the master switch off gives
    // no flag; bit 1 alone is only its Waves option)
    let legacy_water = water_fx.counts.z == 0u;
    var wave_ibl = wavef * frame.water_normal.xyz;
    wave_ibl.z *= 2.0;
    let legacy_reflects = length(wave_ibl) > 1e-6;
    var legacy_glint_n = n;
    if (legacy_water) {
        let vdu = clamp(-view_vec.z * 2.0, 0.0, 1.0);
        let up_bias = max(dist, 32.0) / 32.0 * (1.0 - vdu);
        legacy_glint_n = water_unit(water_unit(wavef) + vec3<f32>(0.0, 0.0, up_bias));
    }

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
    let df2 = max(vec2<f32>(0.0), vec2<f32>(df3.x + df3.y + df3.z, dot(view_vec, water_unit(wavef)) * fs + fo));

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
    var r = reflect(view_vec, normalize(mix(vec3<f32>(0.0, 0.0, 1.0), n, 0.6)));
    // the direction the sky and the probes are read along: kept above the
    // horizon for Aurora's softened normal; along wave_ibl's ray as it is,
    // as doProbeSample reads it. A steep facet then reflects the probe's
    // lower half, not the sky flipped up, which put a second sun on the
    // water beside the glint (the Legacy Viewer shows one)
    var sky_dir = vec3<f32>(r.x, r.y, abs(r.z));
    if (legacy_water && legacy_reflects) {
        r = reflect(view_vec, normalize(wave_ibl));
        sky_dir = r;
    }
    var radiance = sky_color(vec3<f32>(sky_dir.x, sky_dir.y, abs(sky_dir.z)));
    if (probes_on()) {
        // without a planar reflection the water reflects its probes
        // as blurred as the EEP asks (doProbeSample: lod = (1 - gloss) x
        // max lod, gloss = 1 - blur_multiplier): a sharp sun disc and haze
        // at lod 0 whitened the surface
        let lod = clamp(frame.water_params.w, 0.0, 1.0) * probe_max_lod();
        let s = sample_probes_water(in.world_pos, in.clip.xy, sky_dir, lod);
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
    if (!legacy_reflects) { radiance = vec3<f32>(0.0); }

    // sun glint (punctual light, roughness = blur multiplier)
    let l = normalize(frame.sun_dir.xyz);
    let vv = -view_vec;
    let h = normalize(l + vv);
    var rough=clamp(frame.water_params.w,0.02,1.0);
    if(water_fx_on(1u)){rough=clamp(rough*0.65+footprint*0.06,0.035,0.25);}
    rough=mix(rough,0.055,smoothstep(0.004,0.045,-wake.z)*0.65);
    let gn = select(n, legacy_glint_n, legacy_water);
    let shadow = shadow_lift(shadow_factor(in.world_pos, vec3<f32>(0.0, 0.0, 1.0), dist));
    // waterF.glsl: punctual = clamp(nl x (diffPunc + specPunc), 0, 10) x
    // sunlit_linear x shadow x atten, metallic 1 (no diffuse), roughness the
    // blur multiplier, F0 the `specular` uniform (LLDrawPoolWater: the
    // light's colour normalized, x (1.5 + 6 x its horizontal part squared),
    // made linear). The sun colour used to multiply the highlight before the
    // clamp: its faint tail became a second, larger sun beside the reflected
    // one (the Legacy Viewer shows one).
    let light_rgb = frame.sky_obj_light.rgb;
    let light_hue = select(vec3<f32>(0.0), light_rgb / max(length(light_rgb), 1e-6), dot(light_rgb, light_rgb) > 0.0);
    let spec_color = srgb_to_linear(light_hue * (1.5 + 6.0 * dot(l.xy, l.xy)));
    let perceptual = select(rough, frame.water_params.w, legacy_water);
    let atm = ll_atmos(in.world_pos - frame.camera_pos.xyz);
    // calcAtmosphericVarsLinear: sunlit made linear only on a sky with a
    // probe ambiance, then x sky_sunlight_scale (1.5)
    let sunlit_linear = select(srgb_to_linear(atm.sunlit), atm.sunlit, frame.sky_ll.x > 0.5) * 1.5;
    let punctual = legacy_water_punctual(gn, vv, l, perceptual, spec_color) * sunlit_linear * shadow * atm.atten;

    var reflectance = min(1.0,df2.x);
    var glint = punctual;
    if (water_fx_on(512u)) {
        // (the glint has its own fresnel, F of pbrPunctual)
        let cos_i = clamp(dot(n,vv),0.0,1.0);
        reflectance = 0.02037+0.97963*pow(1.0-cos_i,5.0);
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
    // waterF.glsl: frag_color = min(vec4(1), max(color, vec4(0))). The
    // surface never goes past white before the exposure, and a value that is
    // not a number does not spread through the bloom.
    color = clamp(select(color, vec3<f32>(0.0), color != color), vec3<f32>(0.0), vec3<f32>(1.0));
    // The upper face is viewed through air. Opaque source water fog was
    // removed above; do not add another fictitious submerged interval here.
    if(camera_underwater()) {
        let fog=water_fog_view(rel,frame.water_normal.w);
        color=color*fog.transmission+fog.color;
    } else {
        // The Legacy air haze, enhanced water or not: waterF.glsl has none of
        // its own, but the water writes depth and doAtmospherics runs after
        // it (renderGeomPostDeferred: "just before post water alpha"), so
        // hazeF.glsl blends over the surface, on every sky. Aurora's classic
        // air fog (a mix to a fog colour) made that colour near white under
        // a hazy, bright classic sky. Through ll_haze, as the objects beside
        // it (hazeF.glsl's blend uses atten.r, ll_haze the whole atten:
        // TASKS.md), with calcAtmosphericVars in hazeF's eye space
        // (ll_atmos_eye): in world space its glow put a second sun on the sea.
        color = ll_haze(color, ll_atmos_eye(in.world_pos - frame.camera_pos.xyz));
    }
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

// ------------------------------------------------------------ rain (prototype)
// Drops are not simulated: each instance has a fixed random place in a box
// around the camera, falls at its own steady speed and wraps around the box,
// so rain reaches as far as the box everywhere the camera looks. Streaks bend
// the scene behind them (scene_tex), reflect the sky at grazing angles and
// glint against the sun; they fade into geometry, near the lens and toward
// the box's edges, so the wrap never shows.

const RAIN_BOX: vec3<f32> = vec3<f32>(44.0, 44.0, 30.0);
// How long the eye smears a drop into a streak (s).
const RAIN_EXPOSURE: f32 = 0.05;
// Drops are never drawn thinner than this many pixels (they fade instead).
const RAIN_MIN_PX: f32 = 1.3;

// AURORA_RAIN_DEBUG=1: the wet pass shows the wetness memory instead
override rain_debug: f32 = 0.0;

// What falls is snow (`RainLook::snow`): the drops pass draws flakes, and
// nothing wets.
fn snowing() -> bool {
    return frame.rain_look.w > 0.25;
}

// While rain melts the lying snow (frame.snow_cover.z bit 1).
fn snow_melting() -> bool {
    return (u32(frame.snow_cover.z + 0.5) & 1u) != 0u;
}

// The puddles are ice: they may freeze (the Ice setting, bit 2) and it
// snows or snow lies.
fn puddles_frozen() -> bool {
    return (u32(frame.snow_cover.z + 0.5) & 2u) != 0u && !snow_melting() && (snowing() || frame.snow_cover.y > 0.05);
}

// How hard it rains, for what water does (the wet ground, puddles, rings,
// splashes): none in snow.
fn rain_liquid() -> f32 {
    return select(frame.rain.x, 0.0, snowing());
}

struct RainOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) side: vec3<f32>,
    @location(3) life: f32,
}

fn rain_hash(n: u32) -> u32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}

fn rain_rand(n: u32) -> f32 {
    return f32(rain_hash(n) & 0x00ffffffu) / 16777216.0;
}

@vertex
fn vs_rain(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> RainOut {
    let seed = instance * 4u;
    let base = vec3<f32>(rain_rand(seed), rain_rand(seed + 1u), rain_rand(seed + 2u)) * RAIN_BOX;
    // a drop's size (mm): mostly 1-3 mm, a few big ones; its speed is the
    // terminal velocity for that size (Atlas et al. 1973), about 4.6 m/s
    // for the smallest drawn here up to 9 m/s for the biggest
    // heavier rain has bigger drops (Marshall-Palmer: the mean size grows
    // with the rate)
    let heavy = frame.rain.x;
    let snow = snowing();
    var diameter = min(0.9 + heavy * 0.6 - log(1.0 - rain_rand(seed + 3u) * 0.999) * (0.5 + heavy), 6.0);
    var speed = (9.65 - 10.3 * exp(-0.6 * diameter));
    // the wind carries the drops (small ones a little more): the slant from
    // the wind now, the place from all it blew so far (a gust bends the
    // rain, no drop jumps)
    var carried = 0.8 + speed * 0.04;
    if (snow) {
        // flakes of 2 to 12 mm (most about 4), bigger in heavier snow,
        // falling 0.5 to 1.5 m/s (aggregates fall a little faster, Locatelli
        // and Hobbs 1974); light, they go with the air
        let k = rain_rand(seed + 3u);
        diameter = (2.0 + (8.0 + heavy * 2.0) * k * k) * frame.rain_look.w;
        speed = (0.5 + 0.08 * min(diameter, 12.0) + rain_rand(seed + 5u) * 0.3);
        carried = 1.0;
    }
    let time = frame.camera_pos.w;
    var velocity = vec3<f32>(frame.rain_wind.zw * carried, -speed);
    // the box is centered on the camera, a little above it; drops keep their
    // world place (parallax) and wrap through its faces
    let cam = frame.camera_pos.xyz;
    let corner = cam - RAIN_BOX * vec3<f32>(0.5, 0.5, 0.35);
    let moved = base + vec3<f32>(frame.rain_wind.xy * carried, -speed * time);
    var pos = corner + (moved - corner - floor((moved - corner) / RAIN_BOX) * RAIN_BOX);
    if (snow) {
        // a flake flutters: it swings round a small circle as it falls (a
        // few to 30 cm, once every 1 to 3 s), wider in the wind's eddies
        let gusty = 1.0 + 0.04 * length(frame.rain_wind.zw);
        let radius = (0.04 + 0.26 * rain_rand(seed + 6u)) * gusty;
        let turn = 6.2831853 / (1.0 + 2.0 * rain_rand(seed + 7u)) * select(1.0, -1.0, rain_rand(seed + 8u) < 0.5);
        let a = time * turn + rain_rand(seed + 9u) * 6.2831853;
        pos += vec3<f32>(cos(a), sin(a), 0.0) * radius;
        velocity += vec3<f32>(-sin(a), cos(a), 0.0) * radius * turn;
    }

    // a streak: the distance fallen in about 1/16 s, a few millimeters wide,
    // turned around its own axis toward the camera
    let axis = normalize(velocity);
    let to_cam = cam - pos;
    let dist = length(to_cam);
    let side = normalize(cross(axis, to_cam / max(dist, 0.001)) + vec3<f32>(1e-5, 0.0, 0.0));
    // a flake is about as long as it is wide, a little smeared by its motion
    let length_m = select(speed * RAIN_EXPOSURE, diameter * 0.001 + length(velocity) * 0.012, snow);
    // a drop's real width; where that is under RAIN_MIN_PX pixels it is
    // drawn that wide but fainter by as much (its coverage), so far rain
    // turns into a faint haze instead of thick lines
    let clip0 = frame.view_proj * vec4<f32>(pos, 1.0);
    let clip1 = frame.view_proj * vec4<f32>(pos + side, 1.0);
    let px_per_m = length((clip1.xy / max(clip1.w, 1e-3) - clip0.xy / max(clip0.w, 1e-3)) * frame.screen.xy * 0.5);
    let real = diameter * 0.001;
    let width = max(real, RAIN_MIN_PX / max(px_per_m, 1e-3)) * 0.5;
    let coverage = clamp(real / (width * 2.0), 0.0, 1.0);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vertex % 6u];
    let world = pos + side * c.x * width + axis * c.y * length_m * 0.5;

    // fades: right in front of the lens, and toward the box's faces
    let local = abs(pos - (cam + RAIN_BOX * vec3<f32>(0.0, 0.0, 0.15))) / (RAIN_BOX * 0.5);
    let edge = max(max(local.x, local.y), local.z);
    var life = smoothstep(0.35, 1.6, dist) * (1.0 - smoothstep(0.62, 0.95, edge)) * pow(coverage, 0.6);

    var out: RainOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.uv = c;
    out.side = side;
    out.life = life;
    return out;
}

// The shelter map is drawn this frame, in the projector map past the
// projectors' (renderer.rs `rain_shelter_map`, features.y its slot + 1).
fn rain_shelter_on() -> bool {
    return frame.features.y != 0u;
}

// Its slot among the projector maps.
fn rain_shelter_slot() -> u32 {
    return frame.features.y - 1u;
}

// Its view-projection.
fn rain_shelter_vp() -> mat4x4<f32> {
    return frame.spot_vp[min(rain_shelter_slot(), 15u)];
}

// Its square in the shadow atlas (texels): xy its corner, z its side, as
// projector_shadows.rs `map_origin` packs it (the first four maps in the
// fourth cascade tile, then rows of four below the atlas's cascades).
fn rain_shelter_tile() -> vec3<f32> {
    let size = f32(textureDimensions(shadow_map).x) * 0.5;
    let half = size * 0.5;
    let slot = rain_shelter_slot();
    if (slot < 4u) {
        return vec3<f32>(size + f32(slot % 2u) * half, size + f32(slot / 2u) * half, half);
    }
    let k = slot - 4u;
    return vec3<f32>(f32(k % 4u) * half, size * 2.0 + f32(k / 4u) * half, half);
}

// How much a point is under cover: 1 under a roof, a tree or the ground, 0
// in the open. The shelter map is drawn while it rains (renderer.rs,
// `rain_shelter_map`), seen straight down; outside it, or
// when it is not drawn, nothing shelters.
fn rain_sheltered(world: vec3<f32>) -> f32 {
    if (!rain_shelter_on()) { return 0.0; }
    let lp = rain_shelter_vp() * vec4<f32>(world, 1.0);
    if (lp.w < 0.5) { return 0.0; }
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0) { return 0.0; }
    let tile = rain_shelter_tile();
    let dims = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / tile.z;
    var open = 0.0;
    let taps = array<vec2<f32>, 4>(vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(-0.5, 0.5), vec2<f32>(0.5, 0.5));
    for (var i = 0; i < 4; i++) {
        let at = clamp(uv + taps[i] * texel, vec2<f32>(0.5 * texel), vec2<f32>(1.0 - 0.5 * texel));
        open += textureSampleCompareLevel(shadow_map, shadow_sampler, (tile.xy + at * tile.z) / dims, ndc.z - 0.0005);
    }
    return 1.0 - open * 0.25;
}

// A drop blown by the wind comes in slanted, so its cover is what its path
// meets, traced back up against the wind, not only what is straight above
// it: a building leaves a dry zone on its lee side (no drop comes out of a
// wall), and the rain reaches a little way in under an overhang on the
// windward side.
// How far a drop drifts sideways for each metre it falls, in the wind now
// (no more than about 70° from the vertical).
fn rain_slant() -> vec2<f32> {
    // a middling drop falls about 6.5 m/s, a flake about 1 m/s, so the wind
    // lays the snow down far more (up to about 80° from the vertical)
    let fall = select(6.5, 1.0, snowing());
    let cap = select(2.75, 5.5, snowing());
    let slope = frame.rain_wind.zw / fall;
    let flat = dot(slope, slope);
    if (flat < 1e-8) { return vec2<f32>(0.0); }
    return slope * min(1.0, cap / sqrt(flat));
}

// How far sideways a drop's path is tested against the cover, point by
// point (m): evenly enough that a wall or a roof's edge cannot fall between
// two (snow, laid down far more by the wind, goes further).
const RAIN_PATH_OUT = array<f32, 6>(0.6, 1.5, 3.0, 5.0, 8.0, 12.0);

// The points of the path: five for rain, six for snow.
fn rain_path_points() -> i32 {
    return select(5, 6, snowing());
}

// How high up the path point `i` is, for a slant of `slope` (m sideways
// per metre fallen); no higher than 12 m.
fn rain_path_up(i: i32, slope: f32) -> f32 {
    return min(RAIN_PATH_OUT[i] / max(slope, 0.05), 12.0);
}

// The height above which the slanted rain reaches at `xy`: the topmost
// surface there, or higher where cover upwind keeps it off (a surface `top`
// high, `dz` up the path, shelters everything below `top - dz` here).
fn rain_reached_height(xy: vec2<f32>, near_z: f32) -> f32 {
    var top = rain_surface_height(xy, near_z);
    let s = rain_slant();
    if (top < -1e8 || dot(s, s) < 0.0025) { return top; }
    let slope = length(s);
    for (var i = 0; i < rain_path_points(); i++) {
        let dz = rain_path_up(i, slope);
        let up = rain_surface_height(xy - s * dz, near_z);
        if (up > -1e8) { top = max(top, up - dz); }
    }
    return top;
}

fn rain_sheltered_slanted(world: vec3<f32>) -> f32 {
    let straight = rain_sheltered(world);
    let s = rain_slant();
    if (dot(s, s) < 0.0025) { return straight; }
    let slope = length(s);
    // walk up the path against the shelter map's topmost surface. It is
    // blocked where it goes in under cover from the open (a wall's side, an
    // overhang's underside), where it rises up through a surface (the next
    // point is above it, but it was higher than the path's last point: the
    // roof is in between), or where it is still under cover at the top.
    // Starting under cover, it blows in only within the first point (0.6 m
    // sideways): seen from above a wall and an open overhang are the same
    // (the topmost surface over both), so deeper in, under a roof, behind a
    // wall or a door, it stays dry; the rain still blows in a little at a
    // doorway or under an awning.
    var under = straight > 0.5;
    var last_z = world.z;
    for (var i = 0; i < rain_path_points(); i++) {
        let dz = rain_path_up(i, slope);
        let p = world + vec3<f32>(-s * dz, dz);
        let top = rain_surface_height(p.xy, p.z);
        let now = top > p.z + 0.05;
        if (now && !under) { return 1.0; }
        if (!now && under && top > last_z) { return 1.0; }
        if (i == 0 && now && under) { return 1.0; }
        under = now;
        last_z = p.z;
    }
    return select(0.0, 1.0, under);
}

// The light local lights give a drop: some all around it, much more when the
// lamp is behind it seen from the camera (the halo of rain around a street
// light). Point lights only; projectors are left out of the prototype.
fn rain_lamps(world: vec3<f32>, frag: vec2<f32>, view: vec3<f32>) -> vec3<f32> {
    var lights = light_walk(world, frag);
    var sum = vec3<f32>(0.0);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world;
        let dist = length(to_l);
        let radius = lt.pos_radius.w;
        if (dist >= radius) { continue; }
        let atten = ll_light_atten(dist / radius, lt.color_falloff.w);
        let behind = max(dot(-view, to_l / max(dist, 1e-4)), 0.0);
        sum += lt.color_falloff.rgb * atten * (0.06 + pow(behind, 8.0) * 1.1);
    }
    return sum;
}

@fragment
fn fs_rain(in: RainOut) -> @location(0) vec4<f32> {
    return rain_streak(in, 0.25);
}

// A drop seen as a streak of water: bends the scene behind, reflects the
// sky, catches the sun and the lamps. `soft`: how far (m) it fades into what
// is behind it.
// A snowflake: a soft white disc lit by the sky, the sun and the lamps
// around (no glass: it scatters the light instead of bending it).
fn snow_flake(in: RainOut, soft: f32) -> vec4<f32> {
    if (in.life <= 0.001) { discard; }
    if (in.world.z < frame.params.y || camera_underwater()) { discard; }
    let d = length(in.uv);
    var alpha = (1.0 - smoothstep(0.35, 1.0, d)) * in.life * 0.85;
    if (alpha <= 0.002) { discard; }
    let view = normalize(frame.camera_pos.xyz - in.world);
    let ambient = frame.sky_zenith.rgb * 0.55 + frame.sky_horizon.rgb * 0.45;
    // lit from all sides; a little brighter seen against the sun (forward
    // scattering through the ice)
    let toward_sun = max(dot(-view, frame.sun_dir.xyz), 0.0);
    let sun = frame.sun_color.rgb * frame.sun_dir.w * (0.3 * max(frame.sun_dir.z, 0.0) + 0.4 * pow(toward_sun, 6.0));
    let lamps = rain_lamps(in.world, in.clip.xy, view);
    let color = ambient * 0.95 + sun + lamps * 0.7;
    alpha *= 1.0 - rain_sheltered_slanted(in.world);
    let suv = screen_uv(in.clip.xy);
    let depth = scene_depth_at(suv);
    if (depth > 0.0) {
        let behind = world_from_depth(suv, depth);
        let cam = frame.camera_pos.xyz;
        alpha *= clamp((distance(behind, cam) - distance(in.world, cam)) / soft, 0.0, 1.0);
    }
    if (alpha < 0.003) { discard; }
    return vec4<f32>(apply_fog(color, in.world) * alpha, alpha);
}

fn rain_streak(in: RainOut, soft: f32) -> vec4<f32> {
    if (snowing()) { return snow_flake(in, soft); }
    if (in.life <= 0.001) { discard; }
    // no rain under the water, nor seen from under it
    if (in.world.z < frame.params.y || camera_underwater()) { discard; }
    // a thin cylinder of water seen from the side: its normal sweeps from
    // the side to the camera across the streak; its ends taper
    let across = in.uv.x;
    let r = 1.0 - across * across;
    if (r <= 0.0) { discard; }
    let view = normalize(frame.camera_pos.xyz - in.world);
    let normal = normalize(in.side * across + view * sqrt(r));
    let taper = 1.0 - smoothstep(0.55, 1.0, abs(in.uv.y));
    let fresnel = 0.02037 + 0.97963 * pow(1.0 - max(dot(normal, view), 0.0), 5.0);

    // what is behind, bent through the drop
    let suv = screen_uv(in.clip.xy);
    let bend = vec2<f32>(dot(normal, frame.cam_right.xyz), -dot(normal, frame.cam_up.xyz)) * 0.012;
    let transmitted = textureSampleLevel(scene_tex, lin_clamp, clamp(suv + bend, vec2<f32>(0.001), vec2<f32>(0.999)), 0.0).rgb;
    // the sky it reflects, and the sun shining through it from behind
    let reflection = reflect(-view, normal);
    let reflected = sky_color(vec3<f32>(reflection.xy, abs(reflection.z)));
    let toward_sun = max(dot(-view, frame.sun_dir.xyz), 0.0);
    let glint = frame.sun_color.rgb * frame.sun_dir.w * (pow(toward_sun, 12.0) * 1.5 + pow(toward_sun, 3.0) * 0.15);
    let ambient = frame.sky_horizon.rgb * 0.12;
    let lamps = rain_lamps(in.world, in.clip.xy, view);
    let color = mix(transmitted * 1.04, reflected, fresnel) + (glint + ambient) * (0.35 + fresnel) + lamps * (0.3 + fresnel * 0.5);

    // drops a lamp lights stand out of the dark
    let lit = min(dot(lamps, vec3<f32>(0.2126, 0.7152, 0.0722)) * 0.12, 0.15);
    var alpha = taper * sqrt(r) * (0.22 + fresnel * 0.55 + lit) * in.life;
    alpha *= 1.0 - rain_sheltered_slanted(in.world);
    // soft where a drop meets the ground or a wall behind it
    let depth = scene_depth_at(suv);
    var over_water = false;
    if (depth > 0.0) {
        let behind = world_from_depth(suv, depth);
        let cam = frame.camera_pos.xyz;
        alpha *= clamp((distance(behind, cam) - distance(in.world, cam)) / soft, 0.0, 1.0);
        over_water = behind.z < frame.params.y - 0.02;
    }
    if (alpha < 0.003) { discard; }
    if (over_water) {
        // the scene copy holds what is under the water there (the water is
        // drawn after it): no bending, only the light the drop adds
        let lit_only = mix(vec3<f32>(0.0), reflected, fresnel) + (glint + ambient) * (0.35 + fresnel) + lamps * (0.3 + fresnel * 0.5);
        return vec4<f32>(apply_fog(lit_only, in.world) * alpha, 0.0);
    }
    return vec4<f32>(apply_fog(color, in.world) * alpha, alpha);
}

// --------------------------------------------------- wet ground (prototype)
// A screen pass over what scene pass A drew: each pixel's place and facing
// come from the depth prepass; where it faces up and the shelter map sees it
// open to the sky, it is drawn wet: darker, with the sky in it. Flat spots of
// a noise pattern collect puddles: mirrors for the sky and the lamps, broken
// by ripple rings.

struct RainWetOut {
    @builtin(position) clip: vec4<f32>,
}

@vertex
fn vs_rain_wet(@builtin(vertex_index) vertex: u32) -> RainWetOut {
    let p = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var out: RainWetOut;
    out.clip = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

// The Weather quality setting: 0 low, 1 medium, 2 high.
fn rain_quality() -> i32 {
    return i32(frame.rain_look.z + 0.5);
}

// How much of the rings a pixel can show: their crests are about 10 cm
// apart, so a pixel covering more than a few cm of the surface (far, or at a
// grazing angle) would turn them into sparkle; they fade out instead.
fn rain_ring_detail(footprint: f32) -> f32 {
    return 1.0 - smoothstep(0.015, 0.045, footprint);
}

// Ripple rings: one drop per 0.6 m cell, every 0.9 s at a random time and
// place; the normal tilts across each ring's expanding crest.
fn rain_ripples_layer(xy: vec2<f32>, time: f32, cell: f32, period: f32, salt: u32) -> vec2<f32> {
    let base = floor(xy / cell);
    var slope = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = base + vec2<f32>(f32(i), f32(j));
            let cell_seed = u32(i32(c.x) * 73856093) ^ u32(i32(c.y) * 19349663) ^ salt;
            // each cell's drops come at their own pace, and each lands
            // somewhere new, bigger or smaller, or not at all (a third of
            // the time): no grid shows
            let own = period * (0.7 + 0.6 * rain_rand(cell_seed + 7u));
            let phase = time / own + rain_rand(cell_seed + 2u);
            let age = fract(phase);
            let seed = cell_seed ^ (u32(i32(floor(phase))) * 2654435761u);
            if (rain_rand(seed + 5u) < 0.33) { continue; }
            let offset = vec2<f32>(rain_rand(seed), rain_rand(seed + 1u));
            let size = 0.5 + rain_rand(seed + 3u);
            let centre = (c + offset) * cell;
            let d = xy - centre;
            let dist = length(d);
            let radius = age * cell * 0.58 * size;
            let edge = (dist - radius) * 28.0;
            let ring = exp(-edge * edge) * (1.0 - age) * min(size, 1.0);
            slope += d / max(dist, 1e-4) * ring * sin((dist - radius) * 60.0);
        }
    }
    return slope;
}

fn rain_ripples(xy: vec2<f32>, time: f32) -> vec2<f32> {
    return rain_ripples_layer(xy, time, 0.6, 0.9, 0u);
}

// A surface of standing water in rain of strength `heavy`: the rings get
// denser as it rains harder (two more layers of them, smaller and quicker,
// come in), and a downpour churns it (a fine, fast chop), so that its
// reflections break up instead of staying a mirror. `detail`: what the
// pixels can show of it (`rain_ring_detail`).
fn rain_water_churn(xy: vec2<f32>, time: f32, heavy: f32, detail: f32) -> vec2<f32> {
    // the layers the Weather quality draws: one at Low, two at Medium
    let quality = rain_quality();
    var slope = rain_ripples_layer(xy, time, 0.6, 0.9, 0u);
    if (quality >= 1 && heavy > 0.25) {
        slope += rain_ripples_layer(xy + vec2<f32>(0.21, 0.37), time, 0.37, 0.7, 0x9e3779b9u) * smoothstep(0.25, 0.65, heavy);
    }
    if (quality >= 2 && heavy > 0.55) {
        slope += rain_ripples_layer(xy + vec2<f32>(0.53, 0.11), time, 0.23, 0.55, 0x85ebca6bu) * smoothstep(0.55, 0.95, heavy);
    }
    let t = time * 9.0;
    let chop = vec2<f32>(
        sin(xy.x * 37.0 + t + sin(xy.y * 23.0 - t * 0.7) * 2.0) + sin(xy.y * 29.0 - t * 1.3),
        sin(xy.y * 41.0 - t + sin(xy.x * 19.0 + t * 0.8) * 2.0) + sin(xy.x * 31.0 + t * 1.1));
    slope += chop * 0.18 * heavy * heavy;
    return slope * detail;
}

// The screen position of `p` and how far behind what the depth prepass
// saw there it is (m, along the view; > 0: behind it). w < 0: off screen.
fn rain_reflect_probe(p: vec3<f32>) -> vec4<f32> {
    let clip = frame.view_proj * vec4<f32>(p, 1.0);
    if (clip.w <= 0.0) { return vec4<f32>(0.0, 0.0, 0.0, -1.0); }
    let ndc = clip.xy / clip.w;
    let suv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0) { return vec4<f32>(0.0, 0.0, 0.0, -1.0); }
    let d = scene_depth_at(suv);
    if (d <= 0.0) { return vec4<f32>(suv, -1e9, 1.0); }
    let cam = frame.camera_pos.xyz;
    return vec4<f32>(suv, distance(p, cam) - distance(world_from_depth(suv, d), cam), 1.0);
}

// What a puddle mirrors along `r`: a march through the depth prepass. A
// hit is where the ray passes from in front of a surface to behind it,
// refined to where it crosses, and only if that surface is thin there;
// a ray that slips behind an object's edge is not the object (each
// object's reflection would grow a fringe, speckled by the ripples).
fn rain_puddle_reflection(world: vec3<f32>, r: vec3<f32>) -> vec3<f32> {
    let cam = frame.camera_pos.xyz;
    let start = distance(world, cam);
    // the Weather quality: no march at Low (the sky only), a shorter one
    // at Medium (12 steps reach about 30 m, 20 about 280 m)
    let quality = rain_quality();
    let steps = select(select(20, 12, quality == 1), 0, quality <= 0);
    var t = 0.2;
    var prev_t = 0.0;
    var prev_gap = -1.0;
    for (var k = 0; k < steps; k++) {
        let probe = rain_reflect_probe(world + r * t);
        if (probe.w < 0.0) { break; }
        let gap = probe.z;
        if (gap > 0.0 && prev_gap <= 0.0) {
            // crossed between prev_t and t: halve down to it
            var lo = prev_t;
            var hi = t;
            var at = probe;
            for (var b = 0; b < 5; b++) {
                let mid = (lo + hi) * 0.5;
                let m = rain_reflect_probe(world + r * mid);
                if (m.w < 0.0) { break; }
                if (m.z > 0.0) { hi = mid; at = m; } else { lo = mid; }
            }
            // and only something no nearer the camera than the puddle: the
            // ray goes away from it, it never meets what stands in front
            // (a leg just before a puddle is on screen where the ray is,
            // only just in front of it)
            let surface = distance(world + r * hi, cam) - at.z;
            if (at.z < 0.08 + hi * 0.02 && surface > start - 0.05) {
                return textureSampleLevel(scene_tex, lin_clamp, at.xy, 0.0).rgb;
            }
        }
        prev_gap = gap;
        prev_t = t;
        t *= 1.25;
    }
    return sky_color(vec3<f32>(r.xy, abs(r.z)));
}

// Lamps in wet ground: a highlight per point light, stretched toward the
// viewer into a streak, as on a wet road at night: the film and the
// ripples tilt it every way, but only tilts along the line of sight move
// the reflection much (`across`, `along`: how far it spreads each way, a
// slope's tangent). Sharp in puddles, softer on damp ground.
fn rain_wet_lamps(world: vec3<f32>, frag: vec2<f32>, n: vec3<f32>, view: vec3<f32>, across: f32, along: f32) -> vec3<f32> {
    var t = view - n * dot(view, n);
    t = select(normalize(t), vec3<f32>(1.0, 0.0, 0.0), dot(t, t) < 1e-6);
    let b = cross(n, t);
    let peak = 0.35 / sqrt(across * along);
    var lights = light_walk(world, frag);
    var sum = vec3<f32>(0.0);
    for (var i = next_light(&lights); i >= 0; i = next_light(&lights)) {
        let lt = frame.lights[i];
        let to_l = lt.pos_radius.xyz - world;
        let dist = length(to_l);
        if (dist >= lt.pos_radius.w) { continue; }
        let l = to_l / max(dist, 1e-4);
        if (dot(l, n) <= 0.0) { continue; }
        let atten = ll_light_atten(dist / lt.pos_radius.w, lt.color_falloff.w);
        let h = normalize(l + view);
        let hn = max(dot(h, n), 1e-3);
        let sx = dot(h, b) / (across * hn);
        let sy = dot(h, t) / (along * hn);
        sum += lt.color_falloff.rgb * atten * exp(-(sx * sx + sy * sy)) * peak;
    }
    return sum;
}

// The facing from the depth: the nearer neighbour on each axis (no seams
// at edges). Flat on each triangle.
fn rain_depth_normal(uv: vec2<f32>, world: vec3<f32>) -> vec3<f32> {
    let px = frame.screen.zw;
    let r0 = world_from_depth(uv + vec2<f32>(px.x, 0.0), scene_depth_at(uv + vec2<f32>(px.x, 0.0))) - world;
    let l0 = world - world_from_depth(uv - vec2<f32>(px.x, 0.0), scene_depth_at(uv - vec2<f32>(px.x, 0.0)));
    let d0 = world_from_depth(uv + vec2<f32>(0.0, px.y), scene_depth_at(uv + vec2<f32>(0.0, px.y))) - world;
    let u0 = world - world_from_depth(uv - vec2<f32>(0.0, px.y), scene_depth_at(uv - vec2<f32>(0.0, px.y)));
    let dx = select(l0, r0, dot(r0, r0) < dot(l0, l0));
    let dy = select(u0, d0, dot(d0, d0) < dot(u0, u0));
    var n = normalize(cross(dx, dy));
    if (dot(n, frame.camera_pos.xyz - world) < 0.0) { n = -n; }
    return n;
}

// The wetness memory (renderer.rs `RainWet`): for each cell of a square
// around the camera (`frame.rain_wet`), the height above which surfaces are
// wet (x: the topmost surface the rain reaches, which cover that stays
// raises only slowly, so that an umbrella passing leaves the ground wet) and
// how wet they are (y: rising while it rains, drying for minutes after).
@group(0) @binding(27) var rain_wetness: texture_2d<f32>;

// One frame of the memory: the last one, moved with its square, and the
// shelter map's topmost surface now.
@fragment
fn fs_rain_wet_update(in: RainWetOut) -> @location(0) vec4<f32> {
    let size = f32(textureDimensions(rain_wetness).x);
    let extent = frame.rain_wet.z;
    let xy = frame.rain_wet.xy + in.clip.xy / size * extent;
    let step = frame.rain_wet_step;
    let dt = max(step.z, 0.0);
    var wet_h = 1e9;
    var amount = 0.0;
    var snow_depth = 0.0;
    var known = false;
    if (step.z >= 0.0) {
        let uv = (xy - step.xy) / extent;
        if (all(uv >= vec2<f32>(0.0)) && all(uv < vec2<f32>(1.0))) {
            let s = textureLoad(rain_wetness, vec2<i32>(uv * size), 0);
            wet_h = s.x;
            amount = s.y;
            snow_depth = s.z;
            known = true;
        }
    }
    // what falls (rain or snow) moves the height it reaches; only rain wets
    let falling = frame.rain.x;
    let heavy = rain_liquid();
    // slanted in the wind: ground in the lee of cover stays dry
    let top = rain_reached_height(xy, frame.camera_pos.z);
    // the snow lies on the topmost surface, and only where the slanted snow
    // reaches it (not in the lee of cover)
    let surface = rain_surface_height(xy, frame.camera_pos.z);
    let seen = top > -1e8 && surface > -1e8;
    // its share of the snow: all of it in the open, less in the lee of
    // cover upwind, the less the deeper the lee (how far above the surface
    // the slanted snow passes there): the slow snow's path is shallow in
    // the wind, so even a low kerb puts metres behind it in the lee, and a
    // town is lee almost everywhere; it fills in toward the open over a
    // metre and a half of lee (behind a wall, little; past a kerb, most).
    // Were the lee kept bare as long as it snows, the shelter map's square
    // stayed bare around me while all beyond it took the region's snow.
    let share = select(1.0, 1.0 - smoothstep(0.05, 1.5, top - surface), seen);
    if (!known) {
        // new here (the camera came this way): as wet, and as snowed on, as
        // everywhere else in the open, as if it had been seen all along
        wet_h = select(-1e9, top, top > -1e8);
        amount = step.w;
        // beyond the shelter map's reach (not seen yet): as everywhere in
        // the open, until a real look
        snow_depth = frame.snow_cover.y * share;
    } else if (wet_h < -1e8 && top > -1e8) {
        // only assumed open so far (it came in beyond the shelter map's
        // reach): its first real look decides, at once
        wet_h = top;
        snow_depth *= share;
    } else if (falling > 0.0 && top > -1e8) {
        if (top < wet_h) {
            // something lower opened to the sky: reached from now
            wet_h = top;
        } else {
            // cover that stays: what is under it dries, slowly
            wet_h += (top - wet_h) * (1.0 - exp(-dt / 30.0));
        }
    }
    if (heavy > 0.0) {
        amount += (1.0 - amount) * (1.0 - exp(-dt * (0.5 + heavy) / 8.0));
    } else if (!puddles_frozen()) {
        // dries after; frozen, the water stays as ice
        amount *= exp(-dt / 120.0);
    }
    // the snow: it builds where it falls (the place's reached surface is
    // its topmost, so cover keeps it off), melts after it stops, the faster
    // in rain, which wets what it leaves
    // it builds slowly (about ten minutes to a full cover at full snow), in
    // the lee up to its share; while it snows it does not melt
    if (snowing() && falling > 0.0) {
        snow_depth += max(share - snow_depth, 0.0) * (1.0 - exp(-dt * falling / 600.0));
        // never less than its share of what lies everywhere in the open
        // (the snow so far: a region's weather gives its whole history, so
        // who comes in late sees what has built up)
        snow_depth = max(snow_depth, frame.snow_cover.y * share);
    } else {
        snow_depth *= exp(-dt / select(900.0, 120.0, snow_melting()));
    }
    return vec4<f32>(wet_h, amount, snow_depth, 1.0);
}

// How wet the surface at `world` is from the memory (0..1, blended between
// cells); -1 outside it.
// The memory's cell at `world`, for AURORA_RAIN_DEBUG: yellow, outside the
// memory; red, only assumed open (seen beyond the shelter map); green, wet
// by the memory, blue, dry (under cover); brighter, the wetter.
fn rain_debug_color(world: vec3<f32>) -> vec4<f32> {
    let extent = frame.rain_wet.z;
    let size = textureDimensions(rain_wetness).x;
    let pos = (world.xy - frame.rain_wet.xy) / extent * f32(size);
    if (extent <= 0.0 || any(pos < vec2<f32>(0.0)) || any(pos >= vec2<f32>(f32(size)))) {
        return vec4<f32>(0.6, 0.6, 0.0, 0.6);
    }
    let s = textureLoad(rain_wetness, vec2<i32>(pos), 0);
    if (s.x < -1e8) { return vec4<f32>(0.7, 0.0, 0.0, 0.6) * (0.4 + 0.6 * s.y); }
    let open = smoothstep(-0.3, -0.05, world.z - s.x);
    return vec4<f32>(vec3<f32>(0.0, open, 1.0 - open) * (0.3 + 0.7 * s.y) * 0.6, 0.6);
}

fn rain_wet_memory(world: vec3<f32>) -> f32 {
    let extent = frame.rain_wet.z;
    if (extent <= 0.0) { return -1.0; }
    let size = textureDimensions(rain_wetness).x;
    let pos = (world.xy - frame.rain_wet.xy) / extent * f32(size) - 0.5;
    if (any(pos < vec2<f32>(0.0)) || any(pos > vec2<f32>(f32(size - 1u)))) { return -1.0; }
    let i = vec2<i32>(floor(pos));
    let f = fract(pos);
    var w = vec4<f32>(0.0);
    let offs = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));
    // (the last row and column: their neighbour clamped, weighted zero)
    let last = vec2<i32>(i32(size) - 1);
    for (var k = 0; k < 4; k++) {
        let s = textureLoad(rain_wetness, min(i + offs[k], last), 0);
        w[k] = smoothstep(-0.3, -0.05, world.z - s.x) * s.y;
    }
    return mix(mix(w.x, w.y, f.x), mix(w.z, w.w, f.x), f.y);
}

// The terrain's heights around the camera (viewer scene/rain_ground.rs):
// `frame.rain` yz its corner, w its size (0: none).
@group(0) @binding(26) var rain_ground: texture_2d<f32>;

// The terrain's height at `xy` (bilinear between half-metre cells); -1e9
// outside the square or the regions.
fn rain_terrain(xy: vec2<f32>) -> f32 {
    let extent = frame.rain.w;
    if (extent <= 0.0) { return -1e9; }
    let uv = (xy - frame.rain.yz) / extent;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return -1e9; }
    let size = i32(textureDimensions(rain_ground).x);
    let pos = clamp(uv * f32(size) - 0.5, vec2<f32>(0.0), vec2<f32>(f32(size - 1)));
    let i = vec2<i32>(floor(pos));
    let j = min(i + vec2<i32>(1), vec2<i32>(size - 1));
    let f = fract(pos);
    let a = textureLoad(rain_ground, i, 0).x;
    let b = textureLoad(rain_ground, vec2<i32>(j.x, i.y), 0).x;
    let c = textureLoad(rain_ground, vec2<i32>(i.x, j.y), 0).x;
    let d = textureLoad(rain_ground, j, 0).x;
    if (min(min(a, b), min(c, d)) < -1e3) { return -1e9; }
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// How deep the terrain's dip at `xy` is: how much lower than the ground
// 1.5 m around it (> 0 in a hollow, < 0 on a hump), and how steep it is
// there (w of the result: rise over run).
fn rain_terrain_shape(xy: vec2<f32>, h: f32) -> vec2<f32> {
    var around = 0.0;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.7853982;
        around += rain_terrain(xy + vec2<f32>(cos(a), sin(a)) * 1.5);
    }
    let gx = rain_terrain(xy + vec2<f32>(0.5, 0.0)) - rain_terrain(xy - vec2<f32>(0.5, 0.0));
    let gy = rain_terrain(xy + vec2<f32>(0.0, 0.5)) - rain_terrain(xy - vec2<f32>(0.0, 0.5));
    return vec2<f32>(around / 8.0 - h, length(vec2<f32>(gx, gy)));
}

// The ground's shape over about 30 cm around a point, from the depth: its
// facing across several triangles (xyz), how much higher the point is than
// around it (w: > 0 on a bump, < 0 in a dip); none (w 1e3) where both
// points of a side are off the ground (holes in foliage, a narrow gap).
// A point around that is on something standing on the ground (a box's side,
// a stem: much steeper than ground can be) is left out, its opposite stands
// for it mirrored, so that objects keep no dry ring around them.
fn rain_area_shape(uv: vec2<f32>, world: vec3<f32>) -> vec4<f32> {
    let cam = frame.camera_pos.xyz;
    let dist = max(distance(world, cam), 0.5);
    let r = clamp(0.3 * frame.screen.y / (1.15 * dist), 2.0, 40.0);
    let o = r * frame.screen.zw;
    var pr = world_from_depth(uv + vec2<f32>(o.x, 0.0), scene_depth_at(uv + vec2<f32>(o.x, 0.0)));
    var pl = world_from_depth(uv - vec2<f32>(o.x, 0.0), scene_depth_at(uv - vec2<f32>(o.x, 0.0)));
    var pd = world_from_depth(uv + vec2<f32>(0.0, o.y), scene_depth_at(uv + vec2<f32>(0.0, o.y)));
    var pu = world_from_depth(uv - vec2<f32>(0.0, o.y), scene_depth_at(uv - vec2<f32>(0.0, o.y)));
    let n0 = rain_depth_normal(uv, world);
    var br = rain_off_ground(world, pr) || rain_off_plane(world, n0, pr);
    var bl = rain_off_ground(world, pl) || rain_off_plane(world, n0, pl);
    var bd = rain_off_ground(world, pd) || rain_off_plane(world, n0, pd);
    var bu = rain_off_ground(world, pu) || rain_off_plane(world, n0, pu);
    // a lopsided pair: the side much higher or lower than the other is on
    // something else (a plank's or a stone's edge, a low box)
    let zr = abs(pr.z - world.z);
    let zl = abs(pl.z - world.z);
    let zd = abs(pd.z - world.z);
    let zu = abs(pu.z - world.z);
    if (!br && !bl) {
        br = zr > zl * 2.0 + 0.02;
        bl = zl > zr * 2.0 + 0.02;
    }
    if (!bd && !bu) {
        bd = zd > zu * 2.0 + 0.02;
        bu = zu > zd * 2.0 + 0.02;
    }
    if ((br && bl) || (bd && bu)) { return vec4<f32>(0.0, 0.0, 1.0, 1e3); }
    if (br) { pr = world * 2.0 - pl; }
    if (bl) { pl = world * 2.0 - pr; }
    if (bd) { pd = world * 2.0 - pu; }
    if (bu) { pu = world * 2.0 - pd; }
    var n = normalize(cross(pr - pl, pd - pu));
    if (dot(n, cam - world) < 0.0) { n = -n; }
    return vec4<f32>(n, world.z - (pr.z + pl.z + pd.z + pu.z) * 0.25);
}

// A point around is not on the same ground: rising or dropping more than
// about 14 degrees (and 2 cm) from the point; ground that water pools on is
// flatter, so a point on something standing on it (in front of it too: a
// mushroom's cap a metre nearer the camera) is left out.
// `p` is off the plane of the surface at `world` (facing `n`): something in
// front of it or behind (a leg, a step), even where it is not much higher
// for how far away it is
fn rain_off_plane(world: vec3<f32>, n: vec3<f32>, p: vec3<f32>) -> bool {
    let d = p - world;
    return abs(dot(d, n)) > 0.03 + 0.08 * length(d);
}

fn rain_off_ground(world: vec3<f32>, p: vec3<f32>) -> bool {
    return abs(p.z - world.z) > 0.25 * length(p.xy - world.xy) + 0.02;
}

// A surface in the rain. `n`: its smooth facing; `porosity` 0 (glass,
// metal, polished stone: water stays on top, a mirror-like film) to 1
// (grass, soil, cloth: water soaks in, darker and barely shinier), after
// Lagarde's wet surfaces; `level_only`: the facing is only known per
// triangle, so only near-level ground is drawn wet (no facets on avatars).
fn rain_wet_surface(clip: vec2<f32>, uv: vec2<f32>, world: vec3<f32>, n: vec3<f32>, porosity: f32, level_only: bool, build_level: f32) -> vec4<f32> {
    let cam = frame.camera_pos.xyz;
    let dist = distance(world, cam);
    // the cheap reasons to leave a pixel first (no water standing, too far,
    // dry here): the area's shape below reads eight depths
    // the standing water: how long and hard it has rained, not the rain now
    let heavy = frame.rain_wet.w;
    let far = frame.rain_look.y;
    if (heavy <= 0.0 || dist > far || (!level_only && build_level < 0.5)) { return vec4<f32>(0.0); }
    // how wet: the memory (wet for a while after the rain, and under cover
    // that only passed), else the shelter now
    var open = rain_wet_memory(world + n * 0.05);
    if (open < 0.0) { open = (1.0 - rain_sheltered_slanted(world + n * 0.08)) * step(1e-4, rain_liquid()); }
    if (open < 0.01) { return vec4<f32>(0.0); }
    let raining = rain_liquid();
    let area = rain_area_shape(uv, world);
    let smooth_area = area.w < 100.0;
    // per triangle facings (no G-buffer): only smooth, near-level ground,
    // judged over the area, not the triangle
    let up = select(smoothstep(0.5, 0.85, build_level), smoothstep(0.95, 0.985, area.z) * select(0.0, 1.0, smooth_area), level_only);
    let reach = up * open * (1.0 - smoothstep(far * 0.75, far, dist));
    if (reach < 0.01) { return vec4<f32>(0.0); }

    let view = normalize(cam - world);
    let base = textureSampleLevel(scene_tex, lin_clamp, uv, 0.0).rgb;
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);

    // puddles: the low spots of a noise pattern, tiny in a drizzle and
    // spreading as the rain gets heavier; water only pools within about 8
    // degrees of level, its surface level whatever the ground below is
    // shaped like, broken only by the ripples
    let shape = water_fx_noise(world.xy * 0.16) * 0.7 + water_fx_noise(world.xy * 0.55) * 0.3;
    // judged over the area: the blobs' shapes, not the triangles', make the
    // edges; water runs off bumps (a rock's top) and gathers in dips
    var puddle = 0.0;
    let terrain = rain_terrain(world.xy);
    // in snow the switch is the lying snow's: the puddles left are ice,
    // with the Ice setting
    let puddles_on = select(frame.rain_look.x > 0.5, puddles_frozen(), snowing());
    if (!puddles_on) {
        // puddles off (the weather settings)
    } else if (terrain > -1e8 && abs(world.z - terrain) < 0.12) {
        // on the terrain: the blobs of the builds' puddles, moved by the
        // heightmap: a dip fills sooner (2 cm deeper counts as much as a
        // tenth of the pattern), a hump later, steep ground never
        let ts = rain_terrain_shape(world.xy, terrain);
        let pool_level = mix(0.78, 0.53, sqrt(heavy));
        let flat_ground = 1.0 - smoothstep(0.12, 0.25, ts.y);
        puddle = smoothstep(pool_level, pool_level + 0.05, shape + clamp(ts.x * 5.0, -0.3, 0.3)) * flat_ground;
        puddle *= smoothstep(0.15, 0.4, open);
    } else {
        // on a build (a floor, a road, a roof): the screen's rules
        let pool_level = mix(0.78, 0.53, sqrt(heavy));
        let dip = clamp(-area.w * 4.0, 0.0, 0.15);
        let flat_area = smoothstep(0.95, 0.98, area.z) * select(0.0, 1.0, smooth_area) * (1.0 - smoothstep(0.0, 0.02, area.w))
            * smoothstep(0.985, 0.996, build_level);
        puddle = smoothstep(pool_level, pool_level + 0.05, shape + dip) * flat_area;
        puddle *= smoothstep(0.15, 0.4, open);
    }

    // damp: how wet each spot is. The cracks hold water first (pixels
    // darker than their surroundings: the gaps between cobbles, the grain
    // of wood), then patches, then all of it as the rain gets heavier.
    let px = frame.screen.zw * 3.0;
    var around = 0.0;
    around += dot(textureSampleLevel(scene_tex, lin_clamp, uv + vec2<f32>(px.x, px.y), 0.0).rgb, lum);
    around += dot(textureSampleLevel(scene_tex, lin_clamp, uv + vec2<f32>(-px.x, px.y), 0.0).rgb, lum);
    around += dot(textureSampleLevel(scene_tex, lin_clamp, uv + vec2<f32>(px.x, -px.y), 0.0).rgb, lum);
    around += dot(textureSampleLevel(scene_tex, lin_clamp, uv + vec2<f32>(-px.x, -px.y), 0.0).rgb, lum);
    around *= 0.25;
    let luma = dot(base, lum);
    let crack = smoothstep(0.05, 0.3, (around - luma) / max(around, 1e-3));
    let patches = water_fx_noise(world.xy * 0.35 + vec2<f32>(17.0, 3.0));
    let soaked = clamp(heavy * 2.2 + crack * 0.6 + (patches - 0.5) * 0.5 - 0.15, 0.0, 1.0);

    // wet things are darker and their colours deeper (the water film stops
    // the light scattering off the surface's roughness), porous ones most
    let darker = base * mix(1.0, mix(0.82, 0.5, porosity), soaked);
    let deeper = max(mix(vec3<f32>(dot(darker, lum)), darker, 1.0 + 0.3 * soaked), vec3<f32>(0.0));
    // a film of water: a sheen of the sky and the lamps, faint on porous
    // ground; nothing is added where the sky would only lighten it into a
    // haze. The film evens out the surface's small bumps: its facing is
    // halfway to level, so the bevels of normal maps do not each catch a
    // highlight (bright outlines around every stone and leaf).
    let film = soaked * mix(0.35, 0.1, porosity);
    let nf = normalize(mix(n, vec3<f32>(0.0, 0.0, 1.0), 0.5));
    let rd = reflect(-view, nf);
    let fresnel_d = 0.02037 + 0.97963 * pow(1.0 - max(dot(nf, view), 0.0), 5.0);
    let sky = sky_color(vec3<f32>(rd.xy, abs(rd.z)));
    let sheen = max(sky * fresnel_d - deeper * 0.5, vec3<f32>(0.0)) * film;
    let glint_across = mix(0.05, 0.12, porosity);
    let glints = rain_wet_lamps(world, clip, nf, view, glint_across, glint_across * 4.0) * film * 0.15;
    var color = deeper + sheen + glints;

    if (puddle > 0.01 && puddles_frozen()) {
        // ice: a still, glassy sheet, sharp reflections a little whitened
        // and bluish, frost at its edges and in patches (its surface rough
        // there), the lamps in it sharp
        let frost_n = vec2<f32>(snow_noise(world.xy * 31.0) - 0.5, snow_noise(world.xy * 29.0 + 7.0) - 0.5);
        let frost = clamp((1.0 - puddle) * 1.6 + smoothstep(0.55, 0.85, snow_noise(world.xy * 4.0)) * 0.6, 0.0, 1.0);
        let ni = normalize(vec3<f32>(frost_n * 0.05 * (0.3 + frost), 1.0));
        let r = reflect(-view, ni);
        let fresnel = 0.02037 + 0.97963 * pow(1.0 - max(dot(ni, view), 0.0), 5.0);
        let mirrored = rain_puddle_reflection(world + vec3<f32>(0.0, 0.0, 0.02), r);
        let ice_tint = vec3<f32>(0.9, 0.95, 1.0);
        let glass = mix(base * 0.4, mirrored, 0.3 + fresnel * 0.6) * ice_tint + rain_wet_lamps(world, clip, ni, view, 0.015, 0.03);
        let rime = (frame.sky_zenith.rgb * 0.6 + frame.sky_horizon.rgb * 0.4) * 0.85;
        let ice = mix(glass, rime, frost * 0.7);
        color = mix(color, ice, puddle);
    } else if (puddle > 0.01) {
        let along = max(abs(dot(view, vec3<f32>(0.0, 0.0, 1.0))), 0.05);
        let footprint = dist * 1.15 * frame.screen.w / along;
        let ripple = rain_water_churn(world.xy, frame.camera_pos.w, raining, rain_ring_detail(footprint)) * puddle * sqrt(raining);
        let np = normalize(vec3<f32>(ripple * 0.06, 1.0));
        let r = reflect(-view, np);
        let fresnel = 0.02037 + 0.97963 * pow(1.0 - max(dot(np, view), 0.0), 5.0);
        let mirrored = rain_puddle_reflection(world + vec3<f32>(0.0, 0.0, 0.02), r);
        let pool = mix(base * 0.3, mirrored, 0.35 + fresnel * 0.6) + rain_wet_lamps(world, clip, np, view, 0.025, 0.06 + 0.14 * raining);
        color = mix(color, pool, puddle);
    }
    let a = reach * max(smoothstep(0.0, 0.15, heavy), puddle);
    return vec4<f32>(color * a, a);
}

// The snow lying at `world` from the memory (0..1, blended between cells);
// none outside it.
fn snow_lying(world: vec3<f32>) -> f32 {
    let extent = frame.rain_wet.z;
    if (extent <= 0.0) { return 0.0; }
    let size = textureDimensions(rain_wetness).x;
    let pos = (world.xy - frame.rain_wet.xy) / extent * f32(size) - 0.5;
    if (any(pos < vec2<f32>(0.0)) || any(pos > vec2<f32>(f32(size - 1u)))) { return 0.0; }
    let i = vec2<i32>(floor(pos));
    let f = fract(pos);
    var w = vec4<f32>(0.0);
    let offs = array<vec2<i32>, 4>(vec2<i32>(0, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));
    for (var k = 0; k < 4; k++) {
        w[k] = textureLoad(rain_wetness, i + offs[k], 0).z;
    }
    let depth = mix(mix(w.x, w.y, f.x), mix(w.z, w.w, f.x), f.y);
    if (depth <= 0.002) { return 0.0; }
    // on the topmost surface the snow landed on (the shelter map's), not
    // above it nor below it: what stands there and is not in the map
    // (avatars, what moves) stays clear, what is under cover too; and the
    // wind turning does not move it (the height the rain reaches does).
    // Beyond the map's reach, the height the memory keeps there. Within
    // it, the texels around the point are weighed smoothly (below), so that
    // the snow lies up to walls, plants and the edges of cover with a soft
    // edge, never in the map's squares
    let st = rain_shelter_texel(world.xy, world.z);
    if (st.z < 0.5) {
        let surface = snow_known_surface(world.xy, i);
        if (surface < -1e8) { return depth; }
        return depth * snow_gate(world.z - surface);
    }
    // The 4 x 4 texels around the point, by a tent two texels wide (31 cm):
    // smooth as the point moves, a texel leaving the window at a weight of
    // nothing
    let sp = st.xy - vec2<f32>(0.5);
    let s0 = vec2<i32>(floor(sp));
    let sf = fract(sp);
    var hs: array<f32, 16>;
    for (var k = 0; k < 16; k++) {
        var h = rain_shelter_height_at(s0 + vec2<i32>(k % 4 - 1, k / 4 - 1));
        if (h < -1e8) {
            // beyond the map's reach, or nothing drawn there (bare land):
            // the height the memory keeps there, else the terrain's
            h = snow_known_surface(world.xy, i);
            if (h < -1e8) { return depth; }
        }
        hs[k] = h;
    }
    // Cover well above the point (a roof, a tree's leaves) takes its share
    // of the sky over the window: under a solid roof none comes through,
    // under leaves the snow is thinner where they are thicker, and a roof's
    // edge fades over the window. Judged texel by texel, each leaf in the
    // map left a bare square of 15.6 cm under a tree (a grid of them)
    var total = 0.0;
    var covered = 0.0;
    var taken = 0.0;
    var counted = 0.0;
    for (var k = 0; k < 16; k++) {
        let d = vec2<f32>(f32(k % 4 - 1), f32(k / 4 - 1)) - sf;
        let w = max(2.0 - abs(d.x), 0.0) * max(2.0 - abs(d.y), 0.0);
        let above = hs[k] - world.z;
        total += w;
        covered += w * smoothstep(0.3, 0.6, above);
        // the surface it lies on, at this point's height: grass and leaves
        // a little above it are left out exactly as the gate fades on that
        // side (as much counted as taken, no hole); one below this point (it
        // stands on a ledge, a plant's top) counts whole against it
        taken += w * snow_gate(-above);
        counted += w * smoothstep(-0.3, -0.05, -above);
    }
    let open = 1.0 - covered / max(total, 1e-6);
    // on smooth ground (level, a slope, a hump), the surface between the
    // four nearest texels' centres, blended: a slope is followed exactly
    // (each texel's own height is flat over its square, a stair on a slope:
    // scales in rows across every slope). The window's weighing is for the
    // jumps (a wall, a ledge, a plant, cover overhead)
    let h00 = hs[5];
    let h10 = hs[6];
    let h01 = hs[9];
    let h11 = hs[10];
    let lo = min(min(h00, h10), min(h01, h11));
    let hi = max(max(h00, h10), max(h01, h11));
    // (20 cm across a texel of 15.6 cm: about 52°, past which the snow
    // slides off anyway)
    // (and the point on it: under a canopy of leaves as level as that, the
    // ground below is found in the window)
    let surface = mix(mix(h00, h10, sf.x), mix(h01, h11, sf.x), sf.y);
    if (hi - lo < 0.2 && surface - world.z < 0.3) {
        return depth * snow_gate(world.z - surface) * open;
    }
    if (counted < 1e-3) { return 0.0; }
    return depth * clamp(taken / counted, 0.0, 1.0) * open;
}

// The surface the snow lies on at `xy` where the shelter map has none: the
// height the memory keeps in its cell `i`, else the terrain's (-1e9: none
// known). Without the terrain, a cell the memory had not seen yet (all of
// them, just after the snow-fill button) laid the ground's snow on whatever
// stood there, avatars too.
fn snow_known_surface(xy: vec2<f32>, i: vec2<i32>) -> f32 {
    let kept = textureLoad(rain_wetness, i, 0).x;
    if (kept > -1e8) { return kept; }
    return rain_terrain(xy);
}

// How much snow a point `at` metres above the topmost surface the snow
// lands on takes: all of it on that surface, none well below it (under
// cover) nor well above it (what stands there and is not in the map).
fn snow_gate(at: f32) -> f32 {
    // (above it, within a few centimetres: the map's surfaces lie on it, a
    // foot standing there does not: it is not in the map, nor any avatar)
    return smoothstep(-0.3, -0.05, at) * (1.0 - smoothstep(0.06, 0.16, at));
}

// Smooth value noise (0..1) for the snow's patches.
fn snow_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = rain_rand(rain_hash(bitcast<u32>(i32(i.x)) * 73856093u ^ bitcast<u32>(i32(i.y)) * 19349663u));
    let b = rain_rand(rain_hash(bitcast<u32>(i32(i.x + 1.0)) * 73856093u ^ bitcast<u32>(i32(i.y)) * 19349663u));
    let c = rain_rand(rain_hash(bitcast<u32>(i32(i.x)) * 73856093u ^ bitcast<u32>(i32(i.y + 1.0)) * 19349663u));
    let d = rain_rand(rain_hash(bitcast<u32>(i32(i.x + 1.0)) * 73856093u ^ bitcast<u32>(i32(i.y + 1.0)) * 19349663u));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Deep snow (experimental, frame.snow_cover.z bit 4).
fn snow_deep() -> bool {
    return (u32(frame.snow_cover.z + 0.5) & 4u) != 0u;
}

// Deep snow's layer under a full cover (m).
const SNOW_DEEP_TOP: f32 = 0.25;

// How high the snow's surface stands at `xy` over what it lies on (m): up
// to SNOW_DEEP_TOP under a full cover, its lumps (Snow 004's height), pressed down
// in the footprints.
fn snow_surface(xy: vec2<f32>, lying: f32, slot: u32, lod: f32) -> f32 {
    var lumps = 0.5;
    if (slot > 0u) { lumps = sample_tex_level(slot, xy / 1.6, lod).w; }
    var pressed = 0.0;
    let extent = frame.snow_trail.z;
    if (extent > 0.0) {
        let uv = (xy - frame.snow_trail.xy) / extent;
        if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
            pressed = textureSampleLevel(snow_trail, lin_clamp, uv, 0.0).r;
        }
    }
    return lying * SNOW_DEEP_TOP * (0.55 + 0.45 * lumps) * (1.0 - 0.9 * pressed);
}

// Snow cover, premultiplied: it lies on what faces up, thinning on slopes
// and gone past about 55° (it slides off: walls and steep roofs stay bare),
// first as a patchy dusting (the noise: grass tips, the high spots), then
// a white blanket; lit by the sky, the sun (with its shadows) and the
// lamps, a few glints toward the sun. `n`: the smooth facing.
fn snow_cover(clip: vec2<f32>, world: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    return snow_cover_lit(clip, world, world, n, n);
}

// `snow_cover` judging where snow lies at `lies_at` by the facing `n` (what
// it lies on) and lighting it at `world` by `lit` (deep snow: its top stands
// some centimetres over the surface the snow lies on, and a footprint's
// walls are steep, but the snow there lies on level ground)
fn snow_cover_lit(clip: vec2<f32>, world: vec3<f32>, lies_at: vec3<f32>, n: vec3<f32>, lit: vec3<f32>) -> vec4<f32> {
    // the screen-space terms (ambient occlusion, contact shadows) were
    // made from the depth under the snow: what it buries would darken it
    screen_surface = false;
    contact_surface = false;
    // "Snow settles" off (in snow, rain_look.x): none drawn
    if (snowing() && frame.rain_look.x < 0.5) { return vec4<f32>(0.0); }
    let depth = snow_lying(lies_at + n * 0.05);
    if (depth <= 0.002) { return vec4<f32>(0.0); }
    let cam = frame.camera_pos.xyz;
    let dist = distance(world, cam);
    // its distance in snow (rain_look.y); twice the puddles' while melting
    let far = select(frame.rain_look.y * 2.0, frame.rain_look.y, snowing());
    let slope = smoothstep(0.55, 0.85, n.z);
    let lying = depth * slope;
    if (lying <= 0.002) { return vec4<f32>(0.0); }
    // the snow's own surface (Snow 004: its normal and height, a tile of
    // 1.6 m), read at the mip the pixel covers: soft lumps and grain
    // the textures' slots: normal and height, colour and roughness
    // (scene/snow_detail.rs packs the second times 4096)
    let packed = u32(frame.snow_cover.w + 0.5);
    let slot = packed % 4096u;
    let color_slot = packed / 4096u;
    var sn = lit;
    var height = 0.5;
    let along = max(abs(dot(normalize(cam - world), n)), 0.08);
    let footprint = dist * 1.15 * frame.screen.w / along;
    let lod = max(log2(footprint / (1.6 / 512.0)), 0.0);
    // Deep snow (experimental): where the view meets the snow's own surface,
    // a few centimetres above what it lies on, its lumps and footprints
    // standing up from it (parallax), not the surface below
    var xy = world.xy;
    // how high the view meets it (1 the top: a hollow or a print is lower)
    var risen = 1.0;
    // (deep snow is a surface of its own now, `vs_snow_mesh`: drawn before
    // this pass, it is what the depth shows here)
    if (slot > 0u) {
        let t = sample_tex_level(slot, xy / 1.6, lod);
        let tn = t.xyz * 2.0 - vec3<f32>(1.0);
        height = t.w;
        // its bumps over the surface's own facing (the snow lies level-ish)
        sn = normalize(n + vec3<f32>(tn.xy * 0.7, 0.0));
    }
    // footprints: the snow pressed down, its walls tilting the facing (a
    // dent about as deep as the snow lies, a few centimetres), the thin
    // snow pressed through to what is under it
    let print = snow_pressed(xy);
    let pressed = print.x * slope;
    if (pressed > 0.001) {
        sn = normalize(sn + vec3<f32>(print.yz * 10.0 * lying, 0.0));
        height = mix(height, 0.0, pressed * 0.6);
    }
    // a dusting fills the hollows first, then covers the bumps
    let patches = snow_noise(xy * 2.3) * 0.65 + snow_noise(xy * 9.1) * 0.35;
    // (the lumps that decide where a dusting lies at a fixed mip: at the
    // view's, the edge would move as the camera turns)
    var lumps_fixed = 0.5;
    if (slot > 0u) { lumps_fixed = sample_tex_level(slot, xy / 1.6, 4.0).w; }
    let gathered = (lying * 1.3 + (0.5 - lumps_fixed) * 0.45 * (1.0 - lying)) * (1.0 - pressed * (1.0 - lying) * 0.85);
    // a dusting (frost, the first snow) is patchy and lets what is under
    // it show; settled snow, from about half its depth, covers it whole
    var cover = max(smoothstep(patches * 0.9, patches * 0.9 + 0.25, gathered), smoothstep(0.4, 0.65, lying));
    cover *= 1.0 - smoothstep(far * 0.8, far, dist);
    if (cover <= 0.002) { return vec4<f32>(0.0); }
    let view = normalize(cam - world);
    let shade = shadow_factor(world, n, dist);
    // the sun as it lights objects (its colour, at most 1), for the glints
    // and the sheen: the sky's own sun colour is far brighter
    let sunlit = frame.sky_obj_light.rgb * frame.sun_dir.w;
    // glints: a few tiny facets of ice turned toward the sun, each a point
    // somewhere in its cell of 1.7 cm (the whole cell lit drew bright
    // squares up close), no smaller than a pixel and as much light whatever
    // its size (far off: a faint shimmer, not a flicker)
    let cells = xy * 60.0;
    let cell = floor(cells);
    let hash = rain_hash(bitcast<u32>(i32(cell.x)) * 2654435761u ^ bitcast<u32>(i32(cell.y)));
    let g = rain_rand(hash);
    let spot = vec2<f32>(rain_rand(rain_hash(hash ^ 0x9e3779b9u)), rain_rand(rain_hash(hash ^ 0x7f4a7c15u))) * 0.6 + 0.2;
    let pixel = footprint * 60.0;
    let radius = max(0.12, pixel * 0.7);
    let point = (1.0 - smoothstep(radius * 0.5, radius, length(fract(cells) - spot))) * min(0.0144 / (radius * radius), 1.0);
    let half = normalize(frame.sun_dir.xyz + view);
    let glint = step(0.985, g) * point * pow(max(dot(sn, half), 0.0), 40.0) * sunlit * shade * 1.5;
    // the hollows a little shaded, the packed snow of a print greyer
    // (deep snow: the lower the view meets it, the more its walls shade it)
    let cavity = (0.86 + 0.14 * height) * (1.0 - 0.4 * pressed) * mix(0.62, 1.0, risen);
    // Snow 004's colour (nearly white, a little blue in the hollows) and
    // roughness: the colour against its mean (about 0.93), so the snow
    // keeps its brightness and only varies; the smoother patches a soft
    // sheen toward the sun
    var albedo = vec3<f32>(1.0);
    var sheen = vec3<f32>(0.0);
    if (color_slot > 0u) {
        let cr = sample_tex_level(color_slot, xy / 1.6, lod);
        albedo = clamp(cr.rgb / 0.93, vec3<f32>(0.6), vec3<f32>(1.15));
        let smooth_ = 1.0 - cr.a;
        sheen = sunlit * shade * pow(max(dot(sn, half), 0.0), mix(12.0, 60.0, smooth_)) * smooth_ * 0.25;
    }
    // lit as the terrain beside it is (the classic sky's atmospherics and
    // haze, or the PBR shading, the lamps with them), its albedo about
    // 0.85: lit by its own light, brighter than all else, it clipped to a
    // flat white and its lumps and colour were lost
    let base = vec3<f32>(0.85) * albedo * cavity;
    var color: vec3<f32>;
    if (is_classic_sky()) {
        let a = ll_atmos(world - cam);
        color = ll_haze(ll_classic_diffuse(base, sn, world, shade, clip, a) + sheen + glint, a);
    } else {
        color = apply_fog(shade_pbr(base, 0.0, 0.85, 1.0, sn, view, world, shade, clip, false) + sheen + glint, world);
    }
    return vec4<f32>(color * cover, cover);
}

// ------------------------------------------------- snow footprints
// Where feet were set down in the lying snow: how deep it is pressed (0..1)
// in a map that follows the camera (frame.snow_trail, 3 cm a texel). Each
// frame the last one is read at the same places, filled in by the snow
// still falling (fresh snow covers the prints in some minutes), and the
// feet set down this frame (frame.snow_stamps) press theirs: a rounded
// sole, heel to toe along the foot's heading.
@group(0) @binding(28) var snow_trail: texture_2d<f32>;

// `oval`: a body part's print, its width over its length (0: a foot's sole).
fn snow_trail_stamp(xy: vec2<f32>, stamp: vec4<f32>, oval: f32) -> f32 {
    let along = vec2<f32>(cos(stamp.z), sin(stamp.z));
    let d = xy - stamp.xy;
    let a = dot(d, along) / max(stamp.w, 0.04);
    let b = dot(d, vec2<f32>(-along.y, along.x)) / max(stamp.w * select(0.42, oval, oval > 0.0), 0.03);
    // a sole: wider at the ball than the heel; a body part: an oval
    let width = select(1.0 + 0.18 * a, 1.0, oval > 0.0);
    let r = sqrt(a * a + (b / width) * (b / width));
    // (a body part's edge softer: a rounded dent, not a cut sole)
    return 1.0 - smoothstep(select(0.75, 0.55, oval > 0.0), 1.0, r);
}

// The region's footprints drawn into the map around my avatar, each its
// quad (instanced, `MAX_SNOW_PRINTS`), the deepest kept where they meet:
// a rounded sole along the foot's heading, as deep as the snow fallen
// since it was pressed has not filled it in (frame.snow_trail.w: the snow
// fallen so far), only where the foot stood on the snow (its sole at most
// a little above what the snow lies on: not a foot in the air).
// A body part's print (b.w its width over its length, b.z how far above the
// surface its underside may be: scene/snow_trail.rs `body_print`): an oval.
struct PrintOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) @interpolate(flat) stamp: vec4<f32>,
    @location(2) @interpolate(flat) depth: f32,
    @location(3) @interpolate(flat) oval: f32,
};

@vertex
fn vs_snow_print(@builtin(vertex_index) vertex: u32, @location(0) a: vec4<f32>, @location(1) b: vec4<f32>) -> PrintOut {
    var out: PrintOut;
    out.stamp = a;
    out.oval = b.w;
    out.depth = exp(-max(frame.snow_trail.w - b.y, 0.0) / 240.0);
    let surface = rain_surface_height(a.xy, b.x + 1.0);
    let above = select(0.45, b.z, b.z > 0.0);
    if (surface > -1e8 && (b.x - surface > above || b.x - surface < -0.5)) { out.depth = 0.0; }
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vertex % 6u];
    let along = vec2<f32>(cos(a.z), sin(a.z));
    let side = vec2<f32>(-along.y, along.x);
    let half_len = max(a.w, 0.04);
    let half_wide = half_len * max(0.75, b.w * 1.25);
    let xy = a.xy + along * c.x * half_len * 1.25 + side * c.y * half_wide;
    out.world = xy;
    let uv = (xy - frame.snow_trail.xy) / frame.snow_trail.z;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, select(1.0, 0.0, out.depth < 0.02));
    return out;
}

@fragment
fn fs_snow_print(in: PrintOut) -> @location(0) vec4<f32> {
    let d = snow_trail_stamp(in.world, in.stamp, in.oval) * in.depth;
    return vec4<f32>(d, 0.0, 0.0, 1.0);
}

// How deep the snow is pressed at `xy` (0..1), and its slope there (the
// dent's walls), from the footprints' map; none outside it.
fn snow_pressed(xy: vec2<f32>) -> vec3<f32> {
    let extent = frame.snow_trail.z;
    if (extent <= 0.0) { return vec3<f32>(0.0); }
    let uv = (xy - frame.snow_trail.xy) / extent;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) { return vec3<f32>(0.0); }
    let texel = 1.0 / f32(textureDimensions(snow_trail).x);
    let c = textureSampleLevel(snow_trail, lin_clamp, uv, 0.0).r;
    let px = textureSampleLevel(snow_trail, lin_clamp, uv + vec2<f32>(texel, 0.0), 0.0).r;
    let mx = textureSampleLevel(snow_trail, lin_clamp, uv - vec2<f32>(texel, 0.0), 0.0).r;
    let py = textureSampleLevel(snow_trail, lin_clamp, uv + vec2<f32>(0.0, texel), 0.0).r;
    let my = textureSampleLevel(snow_trail, lin_clamp, uv - vec2<f32>(0.0, texel), 0.0).r;
    return vec3<f32>(c, (px - mx) * 0.5, (py - my) * 0.5);
}

// Snow over what the wet pass drew there (premultiplied, over).
fn with_snow(wet: vec4<f32>, snow: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(snow.rgb + wet.rgb * (1.0 - snow.a), snow.a + wet.a * (1.0 - snow.a));
}

// Without the G-buffer (screen-space reflections off): the depth's facing,
// near-level ground only, a middling porosity.
@fragment
fn fs_rain_wet(in: RainWetOut) -> @location(0) vec4<f32> {
    let uv = screen_uv(in.clip.xy);
    let depth = scene_depth_at(uv);
    if (depth <= 0.0) { discard; }
    let world = world_from_depth(uv, depth);
    if (world.z < frame.params.y) { discard; }
    if (rain_debug > 0.5) { return rain_debug_color(world); }
    if (on_deep_snow(world)) { discard; }
    let n = rain_depth_normal(uv, world);
    let out = with_snow(rain_wet_surface(in.clip.xy, uv, world, n, 0.6, true, 1.0), snow_cover(in.clip.xy, world, n));
    if (out.a < 0.01) { discard; }
    return out;
}

// With the G-buffer the screen-space reflections read (object.wgsl
// `gbuffer`: the smooth, normal-mapped facing, roughness, metallic): the
// real shape and material of every surface. Where nothing wrote it (its
// clear value: up, rough, not metallic), the depth's facing as above.
@group(3) @binding(0) var rain_gbuf: texture_2d<f32>;

fn rain_oct_decode(e_in: vec2<f32>) -> vec3<f32> {
    let e = e_in * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) {
        let s = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
        n = vec3<f32>((vec2<f32>(1.0) - abs(n.yx)) * s, n.z);
    }
    return normalize(n);
}

// How level a build is all around a point: the least upward of the smooth
// facings (G-buffer) at the point and at eight points about 15 and 30 cm
// around it. A floor faces up everywhere; a log, a rock's top or a mound
// curves away within a few centimetres. Points on something standing on
// the floor (much higher or lower) are left out, as in `rain_area_shape`.
fn rain_build_level(uv: vec2<f32>, world: vec3<f32>, n: vec3<f32>) -> f32 {
    let dist = max(distance(world, frame.camera_pos.xyz), 0.5);
    let r = clamp(0.3 * frame.screen.y / (1.15 * dist), 2.0, 40.0);
    // the median of the facings around (how level), not the least level
    // nor the mean: a normal map's bevels (the edges of every tile, glyph
    // and plank) catch a sample or two here and there, and either would
    // stamp copies of the pattern, offset by the samples' reach, into the
    // puddles; a curved build (a log) tilts most of them. Water lies flat
    // on the surface, whatever its texture shows.
    var zs: array<f32, 9>;
    zs[0] = n.z;
    var count = 1;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.7853982;
        let reach = select(r, r * 0.5, (k & 1) == 1);
        let at = uv + vec2<f32>(cos(a), sin(a)) * reach * frame.screen.zw;
        let d = scene_depth_at(at);
        if (d <= 0.0) { continue; }
        let p = world_from_depth(at, d);
        if (rain_off_ground(world, p)) { continue; }
        let g = textureLoad(rain_gbuf, vec2<i32>(at * frame.screen.xy), 0);
        if (abs(g.x - 0.5) < 0.004 && abs(g.y - 0.5) < 0.004 && g.z > 0.996 && g.w < 0.004) { continue; }
        var gn = rain_oct_decode(g.xy);
        if (dot(gn, frame.camera_pos.xyz - p) < 0.0) { gn = -gn; }
        // insert, keeping them sorted
        var j = count;
        while (j > 0 && zs[j - 1] > gn.z) {
            zs[j] = zs[j - 1];
            j--;
        }
        zs[j] = gn.z;
        count++;
    }
    return zs[count / 2];
}

@fragment
fn fs_rain_wet_g(in: RainWetOut) -> @location(0) vec4<f32> {
    let uv = screen_uv(in.clip.xy);
    let depth = scene_depth_at(uv);
    if (depth <= 0.0) { discard; }
    let world = world_from_depth(uv, depth);
    if (world.z < frame.params.y) { discard; }
    if (rain_debug > 0.5) { return rain_debug_color(world); }
    if (on_deep_snow(world)) { discard; }
    let g = textureLoad(rain_gbuf, vec2<i32>(in.clip.xy), 0);
    let cleared = abs(g.x - 0.5) < 0.004 && abs(g.y - 0.5) < 0.004 && g.z > 0.996 && g.w < 0.004;
    var out: vec4<f32>;
    if (cleared) {
        let flat_n = rain_depth_normal(uv, world);
        out = with_snow(rain_wet_surface(in.clip.xy, uv, world, flat_n, 0.6, true, 1.0), snow_cover(in.clip.xy, world, flat_n));
    } else {
        var n = rain_oct_decode(g.xy);
        if (dot(n, frame.camera_pos.xyz - world) < 0.0) { n = -n; }
        // rough reads as porous; metal never soaks anything up
        let porosity = clamp((g.z - 0.15) / 0.7, 0.0, 1.0) * (1.0 - g.w);
        // the median around says whether a surface is level, but only one
        // that is itself near level: a leg in front of the ground is mostly
        // ground around it. Its own facing is the G-buffer's, smooth across
        // the triangles (the depth's normal is flat on each, and drew the
        // triangles of avatars and round meshes); with some room, so that a
        // normal map's bevels on level ground do not punch holes
        // (only where water may stand: some standing, a facing near level;
        // a wall needs no eight samples to be told it holds none)
        var level = 0.0;
        if (frame.rain_wet.w > 0.0 && n.z + 0.1 >= 0.5) {
            level = min(rain_build_level(uv, world, n), n.z + 0.1);
        }
        out = with_snow(rain_wet_surface(in.clip.xy, uv, world, n, porosity, false, level), snow_cover(in.clip.xy, world, n));
    }
    if (out.a < 0.01) { discard; }
    return out;
}

// ------------------------------------------------- rain on water (prototype)
// The puddles' rings (`rain_ripples`) as a slope added to the water's waves
// (`water_pixel`): open to the sky, within reach, above the water.
fn rain_water_rings(world: vec3<f32>, footprint: f32) -> vec2<f32> {
    let heavy = rain_liquid();
    if (heavy <= 0.0) { return vec2<f32>(0.0); }
    let reach = 1.0 - smoothstep(30.0, 45.0, distance(world, frame.camera_pos.xyz));
    if (reach <= 0.0) { return vec2<f32>(0.0); }
    let open = 1.0 - rain_sheltered_slanted(world + vec3<f32>(0.0, 0.0, 0.1));
    return rain_water_churn(world.xy, frame.camera_pos.w, heavy, rain_ring_detail(footprint)) * (0.35 * sqrt(heavy) * reach * open);
}


// ------------------------------------------------------ splashes (prototype)
// Each instance is a place where drops keep landing, one at a time: every
// period it lands at a new random spot of a square around the camera
// (fixed in the world, like the drops' box) and throws a few droplets up and
// out, which fall back under gravity within a quarter of a second. Where it
// lands is the top of whatever is there, read from the shelter map (a roof,
// an awning, a table, the ground), or the water if that is higher.

const SPLASH_AREA: f32 = 36.0;
const SPLASH_DROPLETS: u32 = 6u;
const SPLASH_LIFE: f32 = 0.28;

fn rain_splash_none() -> RainOut {
    var out: RainOut;
    out.clip = vec4<f32>(2.0, 2.0, 0.5, 1.0);
    out.world = vec3<f32>(0.0);
    out.uv = vec2<f32>(0.0);
    out.side = vec3<f32>(1.0, 0.0, 0.0);
    out.life = 0.0;
    return out;
}

// Height (m) of the topmost surface at `xy` in the shelter map; none when
// the map is not there or sees nothing below.
// The shelter map's texel under `xy`, as a texel position (xy, the texel
// containing it at its floor) in the shadow atlas; z 1 when it is in the
// map, 0 when not.
fn rain_shelter_texel(xy: vec2<f32>, near_z: f32) -> vec3<f32> {
    if (!rain_shelter_on()) { return vec3<f32>(0.0); }
    let m = rain_shelter_vp();
    let lp = m * vec4<f32>(xy, near_z, 1.0);
    if (lp.w < 0.5) { return vec3<f32>(0.0); }
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) { return vec3<f32>(0.0); }
    let tile = rain_shelter_tile();
    return vec3<f32>(tile.xy + uv * tile.z, 1.0);
}

// The height of the topmost surface in the shelter map's texel `at` (kept
// within the map's square of the atlas), -1e9 where nothing was drawn.
fn rain_shelter_height_at(at: vec2<i32>) -> f32 {
    let m = rain_shelter_vp();
    let tile = rain_shelter_tile();
    let lo = vec2<i32>(tile.xy);
    let hi = vec2<i32>(tile.xy + vec2<f32>(tile.z)) - vec2<i32>(1);
    let d = textureLoad(shadow_map, clamp(at, lo, hi), 0);
    if (d >= 0.999) { return -1e9; }
    return (d - m[3].z) / m[2].z;
}

fn rain_surface_height(xy: vec2<f32>, near_z: f32) -> f32 {
    if (!rain_shelter_on()) { return -1e9; }
    let m = rain_shelter_vp();
    let lp = m * vec4<f32>(xy, near_z, 1.0);
    if (lp.w < 0.5) { return -1e9; }
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) { return -1e9; }
    let tile = rain_shelter_tile();
    // within the shelter map's own square, not the whole atlas (a texel past
    // its edge is another map's)
    let lo = vec2<i32>(tile.xy);
    let hi = vec2<i32>(tile.xy + vec2<f32>(tile.z)) - vec2<i32>(1);
    let at = clamp(vec2<i32>(tile.xy + uv * tile.z), lo, hi);
    let d = textureLoad(shadow_map, at, 0);
    if (d >= 0.999) { return -1e9; }
    // the map's depth is m[2].z * z + m[3].z (seen straight down)
    return (d - m[3].z) / m[2].z;
}

// Snow: a flake that lands lies on the surface and melts away in one to
// two and a half seconds, as rain splashes: as many as the snow brings
// down (renderer.rs: about two a square metre in heavy snow). None in the
// lee of cover, nor on the water (it melts at once).
fn snow_landed(vertex: u32, instance: u32) -> RainOut {
    let j = vertex / 6u;
    if (j > 0u) { return rain_splash_none(); }
    let seed0 = rain_hash(instance * 7u + 2000003u);
    let life_s = 1.0 + rain_rand(seed0) * 1.5;
    let phase = frame.camera_pos.w / life_s + rain_rand(seed0 + 1u);
    let age = fract(phase) * life_s;
    let seed = rain_hash(seed0 ^ (u32(floor(phase)) * 2654435761u));
    let cam = frame.camera_pos.xyz;
    let area = 24.0;
    let base = vec2<f32>(rain_rand(seed), rain_rand(seed + 1u)) * area;
    let corner = cam.xy - vec2<f32>(area * 0.5);
    let xy = corner + (base - corner - floor((base - corner) / area) * area);
    let surface = rain_surface_height(xy, cam.z);
    if (surface < -1e8 || surface < frame.params.y || abs(surface - cam.z) > 40.0) { return rain_splash_none(); }
    if (rain_reached_height(xy, cam.z) > surface + 0.05) { return rain_splash_none(); }
    // indoors, none on the roofs above (a roof drawn blended does not hide
    // what lies on it). The cover over the camera from the map's heights:
    // the vertex stage has no comparison sampler (rain_sheltered), and the
    // splash pipeline, rain's too, fails without one
    let over_camera = rain_surface_height(cam.xy, cam.z);
    if (surface > cam.z && over_camera > cam.z + 0.3) { return rain_splash_none(); }
    let k = rain_rand(seed + 3u);
    let real = (2.0 + 8.0 * k * k) * frame.rain_look.w * 0.001;
    let pos = vec3<f32>(xy, surface + 0.02);
    let to_cam = cam - pos;
    let dist = length(to_cam);
    if (dist > 16.0) { return rain_splash_none(); }
    let a = rain_rand(seed + 4u) * 6.2831853;
    let side = vec3<f32>(cos(a), sin(a), 0.0);
    let ahead = vec3<f32>(-sin(a), cos(a), 0.0);
    let clip0 = frame.view_proj * vec4<f32>(pos, 1.0);
    let clip1 = frame.view_proj * vec4<f32>(pos + side, 1.0);
    let px_per_m = length((clip1.xy / max(clip1.w, 1e-3) - clip0.xy / max(clip0.w, 1e-3)) * frame.screen.xy * 0.5);
    let width = max(real * 0.5, RAIN_MIN_PX / max(px_per_m, 1e-3) * 0.5);
    let coverage = clamp(real / (width * 2.0), 0.0, 1.0);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vertex % 6u];
    let world = pos + side * c.x * width + ahead * c.y * width;
    var out: RainOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.uv = c;
    out.side = side;
    // it lands at once, then melts: smaller and fainter
    out.life = smoothstep(0.0, 0.06, age) * (1.0 - smoothstep(0.25 * life_s, life_s, age))
        * (1.0 - smoothstep(10.0, 16.0, dist)) * smoothstep(0.3, 1.0, dist) * pow(coverage, 0.6);
    return out;
}

@vertex
fn vs_rain_splash(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> RainOut {
    if (snowing()) { return snow_landed(vertex, instance); }
    let heavy = frame.rain.x;
    let seed0 = rain_hash(instance * 7u + 1000003u);
    let period = 0.6 + rain_rand(seed0) * 0.6;
    let phase = frame.camera_pos.w / period + rain_rand(seed0 + 1u);
    let age = fract(phase) * period;
    if (age > SPLASH_LIFE) { return rain_splash_none(); }
    let seed = rain_hash(seed0 ^ (u32(floor(phase)) * 2654435761u));

    // where it lands this time: fixed in the world, wrapped around the camera
    let cam = frame.camera_pos.xyz;
    let base = vec2<f32>(rain_rand(seed), rain_rand(seed + 1u)) * SPLASH_AREA;
    let corner = cam.xy - vec2<f32>(SPLASH_AREA * 0.5);
    let xy = corner + (base - corner - floor((base - corner) / SPLASH_AREA) * SPLASH_AREA);
    let surface = rain_surface_height(xy, cam.z);
    let ground = max(surface, frame.params.y);
    if (ground < -1e8 || abs(ground - cam.z) > 40.0) { return rain_splash_none(); }
    // none in the lee of cover, where the slanted rain does not reach
    if (surface > -1e8 && rain_reached_height(xy, cam.z) > surface + 0.05) { return rain_splash_none(); }

    // one droplet: thrown up and out (harder in heavier rain, its drops are
    // bigger), then falling back
    let j = vertex / 6u;
    let ds = seed + 10u + j * 5u;
    let angle = rain_rand(ds) * 6.2831853;
    let force = 0.6 + 0.6 * heavy;
    let outward = (0.35 + rain_rand(ds + 1u) * 0.75) * force;
    let upward = (0.7 + rain_rand(ds + 2u) * 1.1) * force;
    let v0 = vec3<f32>(cos(angle) * outward, sin(angle) * outward, upward);
    let pos = vec3<f32>(xy, ground + 0.01) + v0 * age + vec3<f32>(0.0, 0.0, -4.9 * age * age);
    if (pos.z < ground) { return rain_splash_none(); }
    let velocity = v0 + vec3<f32>(0.0, 0.0, -9.8 * age);
    let to_cam = cam - pos;
    let dist = length(to_cam);
    if (dist > 20.0) { return rain_splash_none(); }

    // a streak, like a drop's, a little shorter
    let speed = max(length(velocity), 0.05);
    let axis = velocity / speed;
    let side = normalize(cross(axis, to_cam / max(dist, 0.001)) + vec3<f32>(1e-5, 0.0, 0.0));
    let clip0 = frame.view_proj * vec4<f32>(pos, 1.0);
    let clip1 = frame.view_proj * vec4<f32>(pos + side, 1.0);
    let px_per_m = length((clip1.xy / max(clip1.w, 1e-3) - clip0.xy / max(clip0.w, 1e-3)) * frame.screen.xy * 0.5);
    let real = (0.6 + rain_rand(ds + 3u) * 0.6) * 0.001;
    let width = max(real, RAIN_MIN_PX / max(px_per_m, 1e-3)) * 0.5;
    let coverage = clamp(real / (width * 2.0), 0.0, 1.0);
    let length_m = max(speed * RAIN_EXPOSURE * 0.6, 0.004);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[vertex % 6u];
    let world = pos + side * c.x * width + axis * c.y * length_m * 0.5;

    var out: RainOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.uv = c;
    out.side = side;
    out.life = (1.0 - age / SPLASH_LIFE) * (1.0 - smoothstep(10.0, 20.0, dist)) * smoothstep(0.3, 1.0, dist)
        * pow(coverage, 0.6) * 1.6;
    return out;
}

@fragment
fn fs_rain_splash(in: RainOut) -> @location(0) vec4<f32> {
    return rain_streak(in, 0.03);
}

// ------------------------------------------------- distant rain (prototype)
// Beyond the drops' box, rain as layers instead of more drops (after ATI's
// ToyShop, Tatarchuk 2006): four cylinders, 22 to 140 m in radius, each with
// thin faint streaks falling straight. The cylinders stand still in the
// world, around an anchor near the camera, so that camming around moves the
// view through the rain, not the rain with the view; two anchors on grids
// 12 m apart, staggered by half, take over from each other as the camera
// goes (crossfaded), so the rain follows without a jump. Toward the horizon
// only: looking down or up, the cylinders' vertical lines would converge on
// screen, and the drops nearby do the work there. A streak is the scene
// behind it a little lighter (the scene copy), as distant rain is. A layer
// is hidden where the scene is nearer than it, fades into the fog, and the
// nearest keeps out of the shelter; a veil growing with the distance stands
// for the drops too fine to see, so that a downpour greys the distance.

const RAIN_FAR_CELL: f32 = 12.0;

// The streaks the four layers around `anchor` add along the view `d`.
fn rain_far_streaks(anchor: vec2<f32>, d: vec3<f32>, scene: f32, pixel: f32, heavy: f32) -> f32 {
    let cam = frame.camera_pos.xyz;
    let time = frame.camera_pos.w;
    // snow: flakes falling about 1 m/s, seen as short dots, laid down far
    // more by the wind, and brighter (white, where rain is clear)
    let snow = snowing();
    let speed = select(7.0, 1.0, snow);
    let lean = select(1.7, 1.6, snow);
    let o = cam.xy - anchor;
    let a = dot(d.xy, d.xy);
    let b = dot(o, d.xy);
    var far = array<f32, 4>(22.0, 40.0, 75.0, 140.0);
    // snow: dots about as big as the flakes seen near (Flake size)
    let flake = select(1.0, frame.rain_look.w, snow);
    var lift = 0.0;
    // the layers the Weather quality draws: the two nearest at Low, three at
    // Medium
    let layers = rain_quality() + 2;
    for (var k = 0; k < layers; k++) {
        let radius = far[k];
        // where the view leaves the cylinder (the camera is inside it)
        let c = dot(o, o) - radius * radius;
        let t = (-b + sqrt(max(b * b - a * c, 0.0))) / max(a, 1e-6);
        var seen = clamp((scene - t * 0.7) / (t * 0.3), 0.0, 1.0);
        if (seen <= 0.0) { continue; }
        let p = cam + d * t;
        if (p.z < frame.params.y) { continue; }
        if (k == 0) { seen *= 1.0 - rain_sheltered(p); }
        // two offset patterns of columns about 7 pixels apart; in some, now
        // and then, a streak about a pixel wide (soft edged), a few metres
        // long (seen through more air), each as faint as its own drop
        let theta = atan2(p.y - anchor.y, p.x - anchor.x);
        let w = pixel * radius * select(1.0, 0.9 + 0.7 * flake, snow);
        let column = w * select(7.0, 5.0, snow);
        let streak_len = select(0.9 * pow(radius / 22.0, 0.7), max(w * 2.2, 0.02 * flake), snow);
        // snow comes in drifts: denser and thinner patches, slowly passing
        var drifts = 1.0;
        if (snow) {
            let q = theta * radius / 9.0 + dot(frame.rain_wind.xy, vec2<f32>(0.11, 0.07));
            drifts = 0.25 + 0.75 * clamp(0.5 + 0.6 * sin(q + 1.7 * f32(k)) * sin(p.z / 5.0 + q * 0.37 + time * 0.05), 0.0, 1.0);
        }
        for (var s = 0; s < 2; s++) {
            // the wind slants each column by its own place: the wind across
            // the cylinder there, kept the same all along the column (a
            // slant read at each pixel bends a streak into an arc, one for
            // the whole frame turns them all as the camera turns). A first
            // look finds the column, the second uses its slant; snow also
            // drifts with the wind, as the near flakes do (a drop falling
            // `speed` blown sideways at `blown` keeps x + z * blown / speed)
            let base_u = theta * radius + f32(s) * column * 0.37;
            let first = floor(base_u / column);
            let at_col = (first + 0.5) * column / radius;
            let tangent = vec2<f32>(-sin(at_col), cos(at_col));
            // flakes far off are seen through a lot of air and sway as they
            // go: their lean is gentler than one flake's
            let slant = clamp(dot(frame.rain_wind.zw, tangent) / speed * select(1.0, 0.45, snow), -lean, lean);
            let drift = select(0.0, dot(frame.rain_wind.xy, tangent), snow);
            let u = base_u + p.z * slant - drift;
            let col = floor(u / column);
            let seed = bitcast<u32>(i32(col)) * 747796405u + u32(k * 2 + s) * 2891336453u + 17u;
            if (rain_rand(seed + 3u) > select(heavy * 0.8 + 0.1, heavy * 0.45 + 0.05, snow)) { continue; }
            var centre = (col + 0.15 + rain_rand(seed) * 0.7) * column;
            if (snow) {
                // each flake sways a little as it falls
                centre += sin(time * (1.5 + 2.0 * rain_rand(seed + 6u)) + rain_rand(seed + 7u) * 6.2831853) * column * 0.12;
            }
            let across = 1.0 - smoothstep(w * 0.2, w * 0.9, abs(u - centre));
            if (across <= 0.0) { continue; }
            let length_here = streak_len * (0.6 + rain_rand(seed + 4u) * 0.8);
            let period = length_here * (2.5 + rain_rand(seed + 1u) * select(4.0, 9.0, snow));
            let at = fract((p.z + time * speed * (1.0 - 0.1 * f32(k))) / period + rain_rand(seed + 2u)) * period;
            let along = smoothstep(0.0, length_here * 0.3, at) * (1.0 - smoothstep(length_here * 0.5, length_here, at));
            let faint = 0.3 + rain_rand(seed + 5u) * 0.7;
            lift += across * along * faint * seen * drifts * (0.06 - 0.01 * f32(k)) * select(1.0, 1.4, snow);
        }
    }
    return lift;
}

@fragment
fn fs_rain_far(in: RainWetOut) -> @location(0) vec4<f32> {
    let heavy = frame.rain.x;
    if (heavy <= 0.0 || camera_underwater()) { discard; }
    let uv = screen_uv(in.clip.xy);
    let cam = frame.camera_pos.xyz;
    let d = normalize(world_from_depth(uv, 0.5) - cam);
    let horizon = 1.0 - smoothstep(0.2, 0.4, abs(d.z));
    if (horizon <= 0.0) { discard; }
    let flat_len = length(d.xy);
    // a pixel's angle at the middle of the screen, the same for every pixel:
    // a layer's columns must keep one width all down the screen, or they
    // drift sideways and the streaks bend into arcs
    let mid = normalize(world_from_depth(vec2<f32>(0.5, 0.5), 0.5) - cam);
    let mid_up = normalize(world_from_depth(vec2<f32>(0.5, 0.5 + frame.screen.w), 0.5) - cam);
    let pixel = max(length(mid_up - mid), 1e-5);
    let depth = scene_depth_at(uv);
    var scene = 1e9;
    if (depth > 0.0) { scene = distance(world_from_depth(uv, depth), cam); }
    let behind = textureSampleLevel(scene_tex, lin_clamp, uv, 0.0).rgb;
    // only where the air along the view is open to the sky: under a roof
    // (a big hall, a glass roof) there is none, through its windows there is.
    // Open as soon as one point along it is (the most open of them): from
    // under a porch or an awning the nearer points are covered while the
    // view goes on out into the rain, and a mean of them dimmed the rain in
    // the shape of the roof, like a shadow cast on it
    var open = 0.0;
    let reach = array<f32, 4>(12.0, 18.0, 26.0, 36.0);
    for (var k = 0; k < 4; k++) {
        if (reach[k] > scene) { break; }
        open = max(open, 1.0 - rain_sheltered(cam + d * reach[k]));
    }
    if (open <= 0.01) { discard; }

    // the two anchors: cell centres of a grid, and of the same grid moved
    // by half a cell; each weighs the more the nearer the camera is to it
    // (by the distances, which change smoothly everywhere: no jump)
    let cell = cam.xy / RAIN_FAR_CELL;
    let anchor_a = (floor(cell) + 0.5) * RAIN_FAR_CELL;
    let anchor_b = floor(cell + 0.5) * RAIN_FAR_CELL;
    let to_a = dot(fract(cell) - 0.5, fract(cell) - 0.5);
    let to_b = dot(fract(cell + 0.5) - 0.5, fract(cell + 0.5) - 0.5);
    let weight_a = to_b / max(to_a + to_b, 1e-6);
    var lift = 0.0;
    if (weight_a > 0.001) { lift += rain_far_streaks(anchor_a, d, scene, pixel, heavy) * weight_a; }
    if (weight_a < 0.999) { lift += rain_far_streaks(anchor_b, d, scene, pixel, heavy) * (1.0 - weight_a); }
    lift *= horizon * open;

    // the veil: drops too fine to see, more the farther the surface behind
    // (smoothly: a step per layer drew an edge where one met the ground)
    let veil_from = 22.0 / max(flat_len, 0.05);
    // snow hides the distance sooner, and whiter
    let snow = snowing();
    let veil_reach = select(90.0, 55.0, snow);
    let veil = (1.0 - exp(-max(scene - veil_from, 0.0) / veil_reach)) * 0.2 * horizon * open * select(1.0, 1.6, snow);
    let a = clamp(lift * pow(heavy, 1.2), 0.0, 0.5) + clamp(veil * heavy * heavy * 1.25, 0.0, select(0.25, 0.4, snow));
    if (a < 0.002) { discard; }
    // the rain's own colour: what is behind it lightened toward the sky,
    // through the fog of where it is
    let fog_at = cam + d * min(scene, 80.0);
    let toward = select(0.25, 0.7, snow);
    let light = select(1.12, 1.25, snow);
    let rain = apply_fog(mix(behind, frame.sky_horizon.rgb, toward) * light, fog_at);
    return vec4<f32>(rain * a, a);
}

// ------------------------------------------------- deep snow's surface
// Deep snow (experimental): the snow as a surface of its own, a grid of
// cells around the camera raised to the topmost surface the shelter map
// sees, plus the snow lying there (`snow_surface`: its lumps, pressed down
// in the footprints). Real geometry: its outline, boots sinking into it,
// avatars behind a drift hidden. Where the surface under it jumps (a
// wall's foot, a kerb), it is too steep, beyond the map or on the water,
// or no snow lies, the grid's triangles are dropped (the flat cover of the
// wet pass is under them).

struct SnowMeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) valid: f32,
    // the height of the surface under the snow (a triangle across a jump
    // of it is dropped; one down a footprint's or an edge's slope is not)
    @location(3) base: f32,
};

// The grid at this Weather quality: x cells a side, y how far it reaches
// from the camera (m). Its cells are fine near the camera (a few
// centimetres: footprints) and grow outward (`snow_mesh_warp`).
fn snow_mesh_grid() -> vec2<f32> {
    let q = rain_quality();
    if (q <= 0) { return vec2<f32>(128.0, 10.0); }
    if (q == 1) { return vec2<f32>(192.0, 13.0); }
    return vec2<f32>(256.0, 15.0);
}

// How much of the grid's spacing is even (the rest grows with the cube):
// cells a quarter as wide at the centre as on average, two and a half
// times at the edge.
const SNOW_MESH_EVEN: f32 = 0.25;

// A grid line (-1..1) to its place (-1..1).
fn snow_mesh_warp(u: vec2<f32>) -> vec2<f32> {
    return SNOW_MESH_EVEN * u + (1.0 - SNOW_MESH_EVEN) * u * u * u;
}

// How deep the snow lies at `xy` in the memory (0..1, no gating: the grid
// stands on the topmost surface, which is where the snow lies).
fn snow_memory_depth(xy: vec2<f32>) -> f32 {
    let extent = frame.rain_wet.z;
    if (extent <= 0.0) { return 0.0; }
    let size = textureDimensions(rain_wetness).x;
    let pos = (xy - frame.rain_wet.xy) / extent * f32(size) - 0.5;
    if (any(pos < vec2<f32>(0.0)) || any(pos > vec2<f32>(f32(size - 1u)))) { return 0.0; }
    let i = vec2<i32>(floor(pos));
    let f = fract(pos);
    let a = textureLoad(rain_wetness, i, 0).z;
    let b = textureLoad(rain_wetness, i + vec2<i32>(1, 0), 0).z;
    let c = textureLoad(rain_wetness, i + vec2<i32>(0, 1), 0).z;
    let d = textureLoad(rain_wetness, i + vec2<i32>(1, 1), 0).z;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The snow's top at `xy`: z the surface under it (-1e9: none to stand on),
// w how high the snow stands on it (m), y how deep it lies (0..1).
fn snow_mesh_top(xy: vec2<f32>, fade: f32) -> vec3<f32> {
    return snow_mesh_top_soft(xy, fade, 0.0);
}

// The same with the footprints read as a cell of `blur` m sees them (five
// taps): a coarse cell takes their mean, not one sharp sample (spikes).
fn snow_mesh_top_soft(xy: vec2<f32>, fade: f32, blur: f32) -> vec3<f32> {
    let surface = rain_surface_height(xy, frame.camera_pos.z);
    if (surface < -1e8 || surface < frame.params.y) { return vec3<f32>(-1e9, 0.0, 0.0); }
    let lying = snow_memory_depth(xy) * fade;
    // as `snow_surface`, without its lumps: the vertex stage cannot read
    // the texture array (they shade the surface in the fragment stage)
    var pressed = 0.0;
    let extent = frame.snow_trail.z;
    if (extent > 0.0) {
        let taps = array<vec2<f32>, 5>(vec2<f32>(0.0), vec2<f32>(0.5, 0.0), vec2<f32>(-0.5, 0.0), vec2<f32>(0.0, 0.5), vec2<f32>(0.0, -0.5));
        for (var k = 0; k < 5; k++) {
            let uv = (xy + taps[k] * blur - frame.snow_trail.xy) / extent;
            if (all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0))) {
                pressed += textureSampleLevel(snow_trail, lin_clamp, uv, 0.0).r * 0.2;
            }
        }
    }
    // standing up only where it lies deep: as it thins (melting in rain),
    // the surface sinks into what it lies on, its footprints with it, and
    // the flat dusting is left (a thin layer kept its trenches, and the
    // puddles filled them). It stands from where the cover is whole
    // (SNOW_MESH_SOLID), so it is never seen through.
    let thick = smoothstep(SNOW_MESH_SOLID, 0.9, lying);
    return vec3<f32>(surface, lying * thick * SNOW_DEEP_TOP * 0.8 * (1.0 - 0.9 * pressed), lying);
}

@vertex
fn vs_snow_mesh(@builtin(vertex_index) vertex: u32) -> SnowMeshOut {
    let g = snow_mesh_grid();
    let n = u32(g.x);
    let reach = g.y;
    let corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u),
        vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
    let quad = vertex / 6u;
    let cell_ij = vec2<u32>(quad % n, quad / n);
    let ij = cell_ij + corners[vertex % 6u];
    let u = vec2<f32>(ij) / f32(n) * 2.0 - vec2<f32>(1.0);
    // centred on my avatar (not the camera: orbiting it leaves the grid
    // still), snapped to the finest cell: it does not swim as I walk
    // (farther, its big cells move a fine cell at a time)
    let cam = frame.camera_pos.xyz;
    let fine = reach * SNOW_MESH_EVEN * 2.0 / f32(n);
    var me = frame.snow_trail_step.xy;
    if (all(me == vec2<f32>(0.0))) { me = cam.xy; }
    let centre = floor(me / fine) * fine;
    let xy = centre + snow_mesh_warp(u) * reach;
    // its spacing here (for the facing)
    let cell = max(reach * (SNOW_MESH_EVEN + 3.0 * (1.0 - SNOW_MESH_EVEN) * max(u.x * u.x, u.y * u.y)) * 2.0 / f32(n), fine);
    // thinning to nothing over its outer part: no step at its edge
    let fade = 1.0 - smoothstep(0.85, 0.98, max(abs(u.x), abs(u.y)));
    let c = snow_mesh_top_soft(xy, fade, cell);
    // the triangle's three corners (the same for each of its vertices,
    // whatever the camera): dropped where the surface under it jumps more
    // than 10 cm across it (a ledge, a roof's edge), not drawn up as a wall
    let first = (vertex % 6u) / 3u * 3u;
    var lowest = 1e9;
    var highest = -1e9;
    for (var k = 0u; k < 3u; k++) {
        let cu = vec2<f32>(cell_ij + corners[first + k]) / f32(n) * 2.0 - vec2<f32>(1.0);
        let b = rain_surface_height(centre + snow_mesh_warp(cu) * reach, cam.z);
        lowest = min(lowest, b);
        highest = max(highest, b);
    }
    let across = lowest > -1e8 && highest - lowest < 0.1;
    var out: SnowMeshOut;
    out.valid = 0.0;
    out.n = vec3<f32>(0.0, 0.0, 1.0);
    out.base = c.x;
    if (c.x < -1e8) {
        out.world = vec3<f32>(xy, cam.z);
        out.clip = frame.view_proj * vec4<f32>(out.world, 1.0);
        return out;
    }
    // a puff, not a slab: toward a jump of the surface under it (a roof's
    // or a ledge's edge, a kerb) within 30 cm, the snow rounds down to
    // nothing at the edge itself (looked for 6, 15 and 30 cm away)
    let dirs = array<vec2<f32>, 4>(vec2<f32>(1.0, 0.0), vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(0.0, -1.0));
    // (and one cell away: the grid's own neighbour, its cells up to 30 cm
    // wide out from the camera, must not be across a jump either)
    let away = array<f32, 4>(0.06, 0.15, 0.3, cell);
    let keep_at = array<f32, 4>(0.0, 0.5, 0.85, 0.0);
    var keep = 1.0;
    for (var k = 0; k < 4; k++) {
        for (var r = 0; r < 4; r++) {
            let t = snow_mesh_top(xy + dirs[k] * away[r], fade);
            if (t.x < -1e8 || abs(t.x - c.x) > 0.1) { keep = min(keep, keep_at[r]); }
        }
    }
    // the facing, from the neighbours one cell away
    var h: array<f32, 4>;
    for (var k = 0; k < 4; k++) {
        let t = snow_mesh_top(xy + dirs[k] * cell, fade);
        h[k] = select(t.x + t.y, c.x + c.y, t.x < -1e8 || abs(t.x - c.x) > 0.12);
    }
    let top = c.x + c.y * keep;
    let nrm = normalize(vec3<f32>(h[1] - h[0], h[3] - h[2], 2.0 * cell));
    // 8 mm above the surface where only a dusting lies: no fighting with it
    out.world = vec3<f32>(xy, top + 0.008);
    out.clip = frame.view_proj * vec4<f32>(out.world, 1.0);
    out.n = nrm;
    out.valid = select(0.0, 1.0, across && c.z > SNOW_MESH_SOLID);
    return out;
}

// How deep the snow lies (0..1) before deep snow's surface stands: where
// the cover is whole (`snow_cover_lit`: settled snow covers all from 0.65).
// Thinner, it is the flat dusting on the ground itself. The surface writes
// the depth, which the wet pass after it reads: a surface partly seen
// through showed the ground half through it, and took the wet pass's wet
// sheen and puddles as if it were the ground (a square of them around me,
// as a dusting lay or as the snow melted in rain).
const SNOW_MESH_SOLID: f32 = 0.65;

// A pixel of deep snow's own surface (`vs_snow_mesh`, drawn opaque before
// the wet pass, which reads its depth as the ground's): within the grid,
// standing over the topmost surface where the snow lies whole. The wet pass
// leaves it alone: it laid its wet sheen and its puddles (ice, in snow) on
// the snow, read as level ground.
fn on_deep_snow(world: vec3<f32>) -> bool {
    if (!snow_deep()) { return false; }
    // the grid's square around my avatar, short of its fading rim
    var me = frame.snow_trail_step.xy;
    if (all(me == vec2<f32>(0.0))) { me = frame.camera_pos.xy; }
    let d = abs(world.xy - me);
    if (max(d.x, d.y) > snow_mesh_grid().y * 0.85) { return false; }
    let surface = rain_surface_height(world.xy, frame.camera_pos.z);
    if (surface < -1e8) { return false; }
    // 8 mm at least: the surface's lift over what it lies on
    let up = world.z - surface;
    return up > 0.005 && up < SNOW_DEEP_TOP + 0.02 && snow_memory_depth(world.xy) > SNOW_MESH_SOLID;
}

@fragment
fn fs_snow_mesh(in: SnowMeshOut) -> @location(0) vec4<f32> {
    // (a triangle across a jump of the surface under it: its corners said so)
    if (in.valid < 0.999) { discard; }
    // where it lies by the level ground under it, lit by its own surface
    let c = snow_cover_lit(in.clip.xy, in.world, vec3<f32>(in.world.xy, in.base), vec3<f32>(0.0, 0.0, 1.0), normalize(in.n));
    // opaque or not drawn at all (what it writes in the depth hides the
    // ground): where it is not whole (far, at the distance's fade), the
    // ground's own snow shows instead
    if (c.a < 0.5) { discard; }
    return vec4<f32>(c.rgb / c.a, 1.0);
}
