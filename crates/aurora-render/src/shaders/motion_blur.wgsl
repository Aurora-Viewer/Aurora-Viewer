// Motion blur, Aurora's own (the Legacy Viewer draws none): each pixel
// gathers the image along the motion around it, as a camera's shutter left
// open for a share of the frame would. A tile map of the longest motion
// (and its 3 x 3 neighbourhood) says how far to look; the samples are
// weighed by depth and by how far each one's own motion reaches, so that a
// moving avatar smears over the still background behind it, and a still
// one in front of a panning view stays sharp. After the reconstruction
// filters of McGuire et al. ("A Reconstruction Filter for Plausible Motion
// Blur", 2012) and Jimenez ("Next Generation Post Processing in Call of
// Duty: Advanced Warfare", 2014), written for Aurora.

@group(0) @binding(0) var t_color: texture_2d<f32>;
@group(0) @binding(1) var t_depth: texture_depth_2d;
// xy: the mesh normal, zw: the pixel's screen motion since the last frame
// in uv, without the jitter (object.wgsl depth_normal); 0 where the
// prepass drew nothing
@group(0) @binding(2) var t_motion: texture_2d<f32>;
@group(0) @binding(3) var t_tiles: texture_2d<f32>;
@group(0) @binding(4) var samp: sampler;

struct Mb {
    // this frame's camera, without the jitter
    inv_vp: mat4x4<f32>,
    // the last frame's, without the jitter
    prev_vp: mat4x4<f32>,
    inv_proj: mat4x4<f32>,
    // x: share of the frame's motion the shutter sees, y: longest streak
    // (pixels of the image), z: samples, w: 1 when the motion target is
    // read (else the camera alone, through the depth)
    params: vec4<f32>,
    // image width, height, tiles across, down
    sizes: vec4<f32>,
};
@group(0) @binding(5) var<uniform> mb: Mb;

const TILE: i32 = 16;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_mb(@builtin(vertex_index) vi: u32) -> Out {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: Out;
    o.clip = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, 1.0 - y);
    return o;
}

fn depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(textureDimensions(t_depth));
    let q = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    return textureLoad(t_depth, q, 0);
}

// The streak of the image's pixel `p` over the shutter (pixels): its own
// motion where the prepass drew it moving, the camera's through the depth
// elsewhere (the sky, the water, what stood still), as taa.wgsl reprojects.
fn velocity(p: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(mb.sizes.xy);
    let q = clamp(p, vec2<i32>(0), size - vec2<i32>(1));
    let uv = (vec2<f32>(q) + 0.5) / mb.sizes.xy;
    // reverse-Z: 0 is infinitely far
    let depth = depth_at(uv);
    var m = vec2<f32>(0.0);
    if (mb.params.w > 0.5) {
        let msize = vec2<i32>(textureDimensions(t_motion));
        let mq = clamp(vec2<i32>(uv * vec2<f32>(msize)), vec2<i32>(0), msize - vec2<i32>(1));
        m = textureLoad(t_motion, mq, 0).zw;
    }
    if (!(depth > 0.0 && any(m != vec2<f32>(0.0)))) {
        let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let wp = mb.inv_vp * vec4<f32>(ndc, max(depth, 1e-7), 1.0);
        let pp = mb.prev_vp * vec4<f32>(wp.xyz / wp.w, 1.0);
        if (pp.w <= 1e-6) {
            return vec2<f32>(0.0);
        }
        let pn = pp.xy / pp.w;
        m = uv - vec2<f32>(pn.x * 0.5 + 0.5, 0.5 - pn.y * 0.5);
    }
    let v = m * mb.sizes.xy * mb.params.x;
    let l = length(v);
    // (a NaN from a degenerate matrix is no motion)
    if (!(l < 1e30)) {
        return vec2<f32>(0.0);
    }
    if (l > mb.params.y) {
        return v * (mb.params.y / l);
    }
    return v;
}

// Distance along the view axis (m) at the image's pixel `p`.
fn view_z(p: vec2<f32>) -> f32 {
    let uv = p / mb.sizes.xy;
    let d = depth_at(uv);
    let q = mb.inv_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
    if (abs(q.w) < 1e-9) {
        return 1e9;
    }
    return max(-q.z / q.w, 1e-3);
}

// The longest streak of the tile (pixels).
@fragment
fn fs_tiles(in: Out) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.clip.xy) * TILE;
    let size = vec2<i32>(mb.sizes.xy);
    var best = vec2<f32>(0.0);
    var best_l = 0.0;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            let px = base + vec2<i32>(x, y);
            if (px.x < size.x && px.y < size.y) {
                let v = velocity(px);
                let l = dot(v, v);
                if (l > best_l) {
                    best = v;
                    best_l = l;
                }
            }
        }
    }
    return vec4<f32>(best, 0.0, 1.0);
}

// The longest streak of the tile and its neighbours: how far a moving
// thing reaches over what surrounds it.
@fragment
fn fs_neighbors(in: Out) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.clip.xy);
    let size = vec2<i32>(textureDimensions(t_tiles));
    var best = vec2<f32>(0.0);
    var best_l = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let q = clamp(p + vec2<i32>(x, y), vec2<i32>(0), size - vec2<i32>(1));
            let v = textureLoad(t_tiles, q, 0).xy;
            let l = dot(v, v);
            if (l > best_l) {
                best = v;
                best_l = l;
            }
        }
    }
    return vec4<f32>(best, 0.0, 1.0);
}

// Interleaved gradient noise (Jimenez 2014): the samples' offset, changing
// from pixel to pixel so that their steps blend into a smooth streak.
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// 1 when `a` is in front of `b` or level with it, fading over a depth
// margin that grows with the distance.
fn in_front(a: f32, b: f32) -> f32 {
    let soft = 0.01 + 0.02 * min(a, b);
    return clamp(1.0 - (a - b) / soft, 0.0, 1.0);
}

// Whether a streak of half length `r` reaches `d` pixels away.
fn reaches(d: f32, r: f32) -> f32 {
    return 1.0 - smoothstep(0.95 * r, 1.05 * r + 0.5, d);
}

@fragment
fn fs_gather(in: Out) -> @location(0) vec4<f32> {
    let size = vec2<i32>(mb.sizes.xy);
    let p = clamp(vec2<i32>(in.clip.xy), vec2<i32>(0), size - vec2<i32>(1));
    let c = textureLoad(t_color, p, 0);
    let tiles = vec2<i32>(textureDimensions(t_tiles));
    let vn = textureLoad(t_tiles, clamp(p / TILE, vec2<i32>(0), tiles - vec2<i32>(1)), 0).xy;
    // half lengths: the shutter is centred on the frame
    let rn = length(vn) * 0.5;
    if (rn < 0.5) {
        return c;
    }
    let vc = velocity(p);
    let rc = length(vc) * 0.5;
    let pc = vec2<f32>(p) + 0.5;
    let zc = view_z(pc);
    let n = max(i32(mb.params.z), 2);
    let nf = f32(n);
    let j = ign(in.clip.xy) - 0.5;
    // Each sample stands for a slab of `rn / n` pixels of what passes over
    // the pixel, seen for that share of its own streak; the pixel's own
    // surface keeps the rest (all of it when it stands still).
    var w_sum = min(1.0, rn / (nf * max(rc, 0.5)));
    var sum = c * w_sum;
    for (var i = 0; i < n; i++) {
        let t = ((f32(i) + 0.5 + j) / nf) * 2.0 - 1.0;
        let o = vn * (0.5 * t);
        let ps = pc + o;
        if (any(ps < vec2<f32>(0.0)) || any(ps >= mb.sizes.xy)) {
            continue;
        }
        let d = length(o);
        let vs = velocity(vec2<i32>(ps));
        let rs = length(vs) * 0.5;
        let zs = view_z(ps);
        // the sample, moving, passes in front of the pixel; or the pixel,
        // moving, leaves the sample behind it uncovered
        let front = in_front(zs, zc) * reaches(d, rs) * rn / (nf * max(rs, 0.5));
        let back = in_front(zc, zs) * reaches(d, rc) * rn / (nf * max(rc, 0.5));
        let w = max(front, back);
        sum += textureSampleLevel(t_color, samp, ps / mb.sizes.xy, 0.0) * w;
        w_sum += w;
    }
    return sum / max(w_sum, 1e-6);
}
