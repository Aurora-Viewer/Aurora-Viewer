// Aurora Weather's debris (debris.rs): leaves, sand, paper and embers blown
// by the wind, simulated on the GPU, one thread a particle. Appended to
// common.wgsl (the frame's uniforms and the shadow atlas, whose shelter map
// gives the topmost surface under each point, seen from straight above).
//
// A particle is four vec4 (debris.rs `STRIDE`), also read as the instance
// vertex buffer of `vs_debris` (debris.wgsl):
//   p: xyz its place (m), w its life (1 new .. 0 gone; <= 0 a free slot)
//   v: xyz its velocity (m/s), w its state (bits, `DS_*`)
//   q: its orientation (a unit quaternion: the card's local x along its
//      length, y across, z its face's normal)
//   s: x its motion's phase (rad), y its size (m), z its age (s), w its seed
//
// The air that carries it: the region's wind (the frame's `rain_wind.zw`)
// stronger higher up, eddies from a curl-noise field (divergence free: the
// particles swirl and do not bunch up), and the shared gust curve
// (`weather_gust`) that lifts them and makes more of them.

struct DebrisParams {
    // xyz the camera, w the frame's step (s)
    camera: vec4<f32>,
    // xy the wind now (m/s), z the gust (0..1), w the clock (s)
    wind: vec4<f32>,
    // x the kind (DK_*), y the frame number, z the pool's size, w flags
    look: vec4<u32>,
    // x a free slot's chance to spawn this frame, y the nearest spawn (m),
    // z the farthest (m), w the size scale
    rates: vec4<f32>,
};

@group(1) @binding(0) var<storage, read_write> debris: array<vec4<f32>>;
@group(1) @binding(1) var<uniform> dp: DebrisParams;
// The canopy map (debris.rs, object.wgsl `fs_canopy`): per texel of the
// shelter map's square x 1 where there is foliage, y the top of it (m).
@group(1) @binding(2) var debris_canopy: texture_2d<f32>;
// The floor under the foliage (object.wgsl `fs_canopy_floor`, and the
// terrain): the depth of the topmost surface that is not foliage, on the
// shelter map's projection; what leaves land on under a tree (the shelter
// map only sees the crown there).
@group(1) @binding(3) var debris_floor: texture_depth_2d;

const DK_LEAVES: u32 = 1u;
const DK_SAND: u32 = 2u;
const DK_PAPER: u32 = 3u;
const DK_EMBERS: u32 = 4u;
const DK_MOTES: u32 = 5u;

// state bits (v.w as u32): the kind (low 4 bits), the motion mode (4 bits),
// the variant (8 bits: shape and colour), landed (bit 16)
const DS_MODE_SHIFT: u32 = 4u;
const DS_VARIANT_SHIFT: u32 = 8u;
const DS_LANDED: u32 = 65536u;

// leaves' motions (picked at random, switched now and then mid-fall)
const MODE_FLUTTER: u32 = 0u;
const MODE_TUMBLE: u32 = 1u;
const MODE_SPIRAL: u32 = 2u;

// debris.rs `FLAG_*`
const FLAG_STREAM: u32 = 1u;
const FLAG_CANOPY: u32 = 2u;

// the shelter map's reach: past it nothing is known of the ground, so no
// debris (debris.rs `REACH`, inside renderer.rs's 40 m half side)
const REACH: f32 = 38.0;

fn dh(n: u32) -> u32 {
    var x = n;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    x ^= x >> 16u;
    return x;
}

fn dr(n: u32) -> f32 {
    return f32(dh(n) & 0xffffffu) / 16777216.0;
}

// --------------------------------------------------------- the shelter map
// (as env.wgsl's rain_shelter_*: the projector map past the projectors',
// features.y its slot + 1)

fn ds_on() -> bool {
    return frame.features.y != 0u;
}

fn ds_vp() -> mat4x4<f32> {
    return frame.spot_vp[min(frame.features.y - 1u, 15u)];
}

fn ds_tile() -> vec3<f32> {
    let size = f32(textureDimensions(shadow_map).x) * 0.5;
    let half = size * 0.5;
    let slot = frame.features.y - 1u;
    if (slot < 4u) {
        return vec3<f32>(size + f32(slot % 2u) * half, size + f32(slot / 2u) * half, half);
    }
    let k = slot - 4u;
    return vec3<f32>(f32(k % 4u) * half, size * 2.0 + f32(k / 4u) * half, half);
}

// The map's uv at `xy` (outside 0..1: past its square).
fn ds_uv(xy: vec2<f32>) -> vec2<f32> {
    let lp = ds_vp() * vec4<f32>(xy, frame.camera_pos.z, 1.0);
    let ndc = lp.xy / max(lp.w, 1e-4);
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}

// The topmost surface at `xy` (m), -1e9 where nothing is known.
fn ds_surface(xy: vec2<f32>) -> f32 {
    if (!ds_on()) { return -1e9; }
    let uv = ds_uv(xy);
    if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) { return -1e9; }
    let tile = ds_tile();
    let lo = vec2<i32>(tile.xy);
    let hi = vec2<i32>(tile.xy + vec2<f32>(tile.z)) - vec2<i32>(1);
    let d = textureLoad(shadow_map, clamp(vec2<i32>(tile.xy + uv * tile.z), lo, hi), 0);
    if (d >= 0.999) { return -1e9; }
    let m = ds_vp();
    return (d - m[3].z) / m[2].z;
}

// The canopy over `xy`: x its top (0: none), y how far under it leaves come
// from.
fn ds_canopy(xy: vec2<f32>) -> vec2<f32> {
    if ((dp.look.w & FLAG_CANOPY) == 0u || !ds_on()) { return vec2<f32>(0.0); }
    let uv = ds_uv(xy);
    if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) { return vec2<f32>(0.0); }
    let size = vec2<f32>(textureDimensions(debris_canopy));
    let c = textureLoad(debris_canopy, vec2<i32>(uv * size), 0).xy;
    if (c.x < 0.5) { return vec2<f32>(0.0); }
    // its top, and leaves from up to 2 m under it
    return vec2<f32>(c.y, 2.0);
}

// The ground for a particle at height `z` over `xy`: the shelter map's top
// surface where it is open above; where something is above it, the floor
// (the topmost surface that is not foliage) if that is below it: only
// foliage was above, and it falls through the crown to the ground under
// it. A roof or a wall above it stays the top (it stops against it).
fn ds_ground(xy: vec2<f32>, z: f32) -> f32 {
    let top = ds_surface(xy);
    if (top <= z + 0.05 || (dp.look.w & FLAG_CANOPY) == 0u || !ds_on()) { return top; }
    let uv = ds_uv(xy);
    if (any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) { return top; }
    let d = textureLoad(debris_floor, vec2<i32>(uv * vec2<f32>(textureDimensions(debris_floor))), 0);
    if (d >= 0.999) { return top; }
    let m = ds_vp();
    let floor = (d - m[3].z) / m[2].z;
    return select(top, floor, floor <= z + 0.05);
}

// The surface's facing at `xy` (from its heights a texel around), for a
// particle at height `z`.
fn ds_normal(xy: vec2<f32>, h: f32, z: f32) -> vec3<f32> {
    let e = 0.35;
    let hx = ds_ground(xy + vec2<f32>(e, 0.0), z);
    let hy = ds_ground(xy + vec2<f32>(0.0, e), z);
    // a step (a wall's top, a kerb): level rather than a cliff
    let dx = select(0.0, hx - h, abs(hx - h) < 0.3);
    let dy = select(0.0, hy - h, abs(hy - h) < 0.3);
    return normalize(vec3<f32>(-dx / e, -dy / e, 1.0));
}

// ------------------------------------------------------------- the air

fn ds_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let n = vec3<u32>(vec3<i32>(i) + vec3<i32>(4096));
    let a = dr(n.x * 73856093u ^ n.y * 19349663u ^ n.z * 83492791u);
    let b = dr((n.x + 1u) * 73856093u ^ n.y * 19349663u ^ n.z * 83492791u);
    let c = dr(n.x * 73856093u ^ (n.y + 1u) * 19349663u ^ n.z * 83492791u);
    let d = dr((n.x + 1u) * 73856093u ^ (n.y + 1u) * 19349663u ^ n.z * 83492791u);
    let e = dr(n.x * 73856093u ^ n.y * 19349663u ^ (n.z + 1u) * 83492791u);
    let g = dr((n.x + 1u) * 73856093u ^ n.y * 19349663u ^ (n.z + 1u) * 83492791u);
    let h = dr(n.x * 73856093u ^ (n.y + 1u) * 19349663u ^ (n.z + 1u) * 83492791u);
    let k = dr((n.x + 1u) * 73856093u ^ (n.y + 1u) * 19349663u ^ (n.z + 1u) * 83492791u);
    return mix(mix(mix(a, b, u.x), mix(c, d, u.x), u.y), mix(mix(e, g, u.x), mix(h, k, u.x), u.y), u.z) * 2.0 - 1.0;
}

fn ds_potential(p: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(ds_noise(p), ds_noise(p + vec3<f32>(31.4, 17.1, 9.2)), ds_noise(p + vec3<f32>(-12.7, 43.3, 27.9)));
}

// The eddies: the curl of a noise potential (no sources or sinks, so the
// debris swirls without gathering), carried with the wind.
fn ds_curl(p: vec3<f32>) -> vec3<f32> {
    let e = 0.15;
    let dx = ds_potential(p + vec3<f32>(e, 0.0, 0.0)) - ds_potential(p - vec3<f32>(e, 0.0, 0.0));
    let dy = ds_potential(p + vec3<f32>(0.0, e, 0.0)) - ds_potential(p - vec3<f32>(0.0, e, 0.0));
    let dz = ds_potential(p + vec3<f32>(0.0, 0.0, e)) - ds_potential(p - vec3<f32>(0.0, 0.0, e));
    return vec3<f32>(dy.z - dz.y, dz.x - dx.z, dx.y - dy.x) / (2.0 * e);
}

// The air's velocity at `p`, `above` metres over the ground under it.
fn ds_air(p: vec3<f32>, above: f32) -> vec3<f32> {
    let gust = dp.wind.z;
    let wind = dp.wind.xy;
    let speed = length(wind);
    // slower near the ground (a log profile, roughly), full by 10 m
    let profile = clamp(log(max(above, 0.05) / 0.05 + 1.0) / log(201.0), 0.15, 1.0);
    // the eddies drift with the wind and churn faster in it
    let t = dp.wind.w;
    let q = p * 0.18 - vec3<f32>(wind * t * 0.18, t * 0.05);
    let eddy = ds_curl(q) * (0.4 + 0.12 * speed) * (0.6 + 0.8 * gust);
    // a gust lifts what it blows (strongest near the ground, where it
    // rolls over obstacles)
    let lift = gust * gust * (0.6 + 0.08 * speed) * (1.0 - clamp(above / 6.0, 0.0, 1.0));
    return vec3<f32>(wind * profile * (0.75 + 0.5 * gust), lift) + eddy;
}

// ------------------------------------------------------------ orientation

fn ds_quat_axis(axis: vec3<f32>, angle: f32) -> vec4<f32> {
    return vec4<f32>(axis * sin(angle * 0.5), cos(angle * 0.5));
}

fn ds_quat_mul(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(a.w * b.xyz + b.w * a.xyz + cross(a.xyz, b.xyz), a.w * b.w - dot(a.xyz, b.xyz));
}

// The rotation taking +z to `n`, then `yaw` about `n`.
fn ds_quat_facing(n: vec3<f32>, yaw: f32) -> vec4<f32> {
    let z = vec3<f32>(0.0, 0.0, 1.0);
    let c = dot(z, n);
    var tilt: vec4<f32>;
    if (c > 0.9999) {
        tilt = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    } else if (c < -0.9999) {
        tilt = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    } else {
        tilt = normalize(vec4<f32>(cross(z, n), 1.0 + c));
    }
    return ds_quat_mul(tilt, ds_quat_axis(z, yaw));
}

// ------------------------------------------------------------------ spawn

struct DsKindParams {
    // how fast it falls through still air (m/s; < 0 it rises)
    terminal: f32,
    // how long it lasts (s)
    life: f32,
    // its size (m)
    size: f32,
    // how high it starts at most above the ground (m)
    height: f32,
};

fn ds_kind(kind: u32) -> DsKindParams {
    switch kind {
        // (sand: it settles within a second or two unless the turbulence
        // and the hops along the ground keep it going)
        case DK_SAND: { return DsKindParams(1.8, 3.0, 0.006, 0.8); }
        case DK_PAPER: { return DsKindParams(1.6, 30.0, 0.22, 1.5); }
        // (buoyant while hot; debris_sim's step makes it fall as it cools)
        case DK_EMBERS: { return DsKindParams(-0.6, 6.0, 0.025, 1.5); }
        // (as light as the air: they hang, close to the camera)
        case DK_MOTES: { return DsKindParams(0.03, 25.0, 0.0025, 2.5); }
        default: { return DsKindParams(1.4, 28.0, 0.09, 12.0); }
    }
}

// A new particle in slot `i` (its seed `seed`), or none (p.w 0) where no
// open ground was found.
fn ds_spawn(i: u32, seed: u32) {
    let kind = dp.look.x;
    let k = ds_kind(kind);
    let cam = dp.camera.xyz;
    // mostly at mid distance, a few close passes: near the camera they are
    // big on screen and few are enough
    var lo = dp.rates.y;
    if (dr(seed + 1u) < 0.06) { lo = 2.5; }
    let r = mix(lo, dp.rates.z, sqrt(dr(seed + 2u)));
    let a = dr(seed + 3u) * 6.2831853;
    var xy = cam.xy + vec2<f32>(cos(a), sin(a)) * r;
    // sand comes from the ground near the camera, streaming past it: start
    // upwind so it crosses the view
    let wind = dp.wind.xy;
    if (kind == DK_SAND && length(wind) > 0.1) {
        xy -= normalize(wind) * r * 0.5;
    }
    let ground = ds_surface(xy);
    if (ground < -1e8) { return; }
    var z = ground + 0.05 + dr(seed + 4u) * k.height;
    var from_canopy = false;
    if (kind == DK_LEAVES) {
        // look for foliage at a few places in the ring: leaves come from
        // the trees around, however few they are
        var canopy = ds_canopy(xy);
        for (var t = 0u; t < 6u && canopy.x <= 0.0; t++) {
            let rt = mix(lo, dp.rates.z, sqrt(dr(seed + 30u + t * 2u)));
            let at = dr(seed + 31u + t * 2u) * 6.2831853;
            let tried = cam.xy + vec2<f32>(cos(at), sin(at)) * rt;
            let c = ds_canopy(tried);
            if (c.x > 0.0) {
                xy = tried;
                canopy = c;
            }
        }
        if (canopy.x > 0.0) {
            // just under the canopy's top, among its leaves
            z = canopy.x - 0.3 - dr(seed + 4u) * min(canopy.y, 2.0);
            from_canopy = true;
        } else if ((dp.look.w & FLAG_STREAM) != 0u && length(wind) > 0.5) {
            // none nearby: a thin stream coming in on the wind, from upwind
            // and above, as from trees out of sight
            xy = cam.xy - normalize(wind) * dp.rates.z * (0.3 + 0.6 * dr(seed + 2u)) + vec2<f32>(-wind.y, wind.x) / length(wind) * (dr(seed + 5u) - 0.5) * 40.0;
            let g = ds_surface(xy);
            if (g < -1e8) { return; }
            z = g + 3.0 + dr(seed + 4u) * 8.0;
            from_canopy = true;
        } else {
            return;
        }
    }
    // (never indoors: what does not fall from a canopy starts on the
    // shelter map's top surface, the open sky above it)
    var mode = MODE_FLUTTER;
    let m = dr(seed + 6u);
    if (kind == DK_LEAVES || kind == DK_PAPER) {
        mode = select(select(MODE_SPIRAL, MODE_TUMBLE, m < 0.7), MODE_FLUTTER, m < 0.45);
    }
    let variant = dh(seed + 7u) & 255u;
    let state = kind | (mode << DS_MODE_SHIFT) | (variant << DS_VARIANT_SHIFT);
    let size = k.size * (0.7 + 0.6 * dr(seed + 8u)) * dp.rates.w;
    let yaw = dr(seed + 9u) * 6.2831853;
    let b = i * 4u;
    debris[b] = vec4<f32>(xy, z, 1.0);
    debris[b + 1u] = vec4<f32>(ds_air(vec3<f32>(xy, z), z - ground) * 0.5, bitcast<f32>(state));
    debris[b + 2u] = ds_quat_facing(vec3<f32>(0.0, 0.0, 1.0), yaw);
    debris[b + 3u] = vec4<f32>(dr(seed + 10u) * 6.2831853, size, 0.0, bitcast<f32>(seed));
}

// ------------------------------------------------------------- simulate

@compute @workgroup_size(64)
fn cs_debris(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= dp.look.z) { return; }
    let b = i * 4u;
    var p = debris[b];
    let dt = dp.camera.w;
    let frame_seed = dh(i * 9781u + dp.look.y * 6271u);
    if (p.w <= 0.0) {
        if (dp.look.x != 0u && dr(frame_seed) < dp.rates.x) {
            ds_spawn(i, dh(frame_seed ^ 0x51ed27u));
        }
        return;
    }
    var v = debris[b + 1u];
    var q = debris[b + 2u];
    var s = debris[b + 3u];
    var state = bitcast<u32>(v.w);
    let kind = state & 15u;
    let k = ds_kind(kind);
    let seed = bitcast<u32>(s.w);
    s.z += dt;

    // gone: too far from the camera (the shelter map moves with it), or
    // the kind changed
    let off = p.xy - dp.camera.xy;
    if (kind != dp.look.x || dot(off, off) > REACH * REACH || p.z < dp.camera.z - 60.0) {
        debris[b] = vec4<f32>(0.0);
        return;
    }

    let ground = ds_ground(p.xy, p.z);
    let landed = (state & DS_LANDED) != 0u;
    var vel = v.xyz;
    if (landed) {
        // on the ground: it skitters when a gust takes it, else it lies
        // and fades
        let air = ds_air(p.xyz + vec3<f32>(0.0, 0.0, 0.3), 0.3);
        let push = length(air.xy) * dp.wind.z;
        let grip = select(3.5, 2.0, kind == DK_PAPER);
        if (push > grip && dr(frame_seed + 11u) < dt * 3.0) {
            // lifted again
            vel = vec3<f32>(air.xy * 0.6, 0.8 + dr(frame_seed + 12u) * 1.5 * dp.wind.z);
            state &= ~DS_LANDED;
        } else if (push > grip * 0.5) {
            // sliding along the ground, turning as it goes
            vel = vec3<f32>(mix(vel.xy, air.xy * 0.25, clamp(dt * 3.0, 0.0, 1.0)), 0.0);
            q = normalize(ds_quat_mul(ds_quat_axis(vec3<f32>(0.0, 0.0, 1.0), (dr(seed + 13u) - 0.5) * length(vel.xy) * dt * 4.0), q));
        } else {
            vel = vec3<f32>(vel.xy * max(1.0 - dt * 4.0, 0.0), 0.0);
        }
        var to = p.xyz + vel * dt;
        let h = ds_ground(to.xy, p.z + 0.25);
        // a wall: it stops against it
        if (h > p.z + 0.25) {
            to = vec3<f32>(p.xy, p.z);
            vel = vec3<f32>(0.0);
        } else if (h > -1e8) {
            to.z = h + 0.004;
        }
        p = vec4<f32>(to, p.w);
        // lying, it fades over its last seconds
        let fade_after = select(14.0, 1.5, kind == DK_SAND || kind == DK_EMBERS);
        if (s.z > fade_after) { p.w -= dt / 4.0; }
    } else {
        let above = p.z - max(ground, p.z - 50.0);
        let air = ds_air(p.xyz, above);
        // drag toward the air's velocity, as fast as its terminal velocity
        // allows: g / vt² (quadratic), a light leaf rides the wind
        let vt = max(abs(k.terminal), 0.2);
        let gravity = vec3<f32>(0.0, 0.0, -9.81 * sign(k.terminal));
        // (the drag is applied after the other forces, implicitly: an
        // explicit step on a quadratic drag overshoots and blows up at a
        // low frame rate, embers below 50 fps)
        let kdrag = 9.81 / (vt * vt);
        var acc = gravity;
        let mode = (state >> DS_MODE_SHIFT) & 15u;
        let rate = 0.7 + 0.6 * dr(seed + 14u);
        if (kind == DK_LEAVES || kind == DK_PAPER) {
            // a leaf's own motions through the air (relative to it): its
            // phase runs faster the faster it falls
            s.x += dt * rate * select(2.6, 1.6, kind == DK_PAPER) * (1.0 + 0.3 * dp.wind.z);
            let glide = normalize(vec3<f32>(cos(dr(seed + 15u) * 6.2831853 + 0.3 * sin(s.z * 0.2)), sin(dr(seed + 15u) * 6.2831853 + 0.3 * sin(s.z * 0.2)), 0.0));
            if (mode == MODE_FLUTTER) {
                // the pendulum: it glides one way then the other, swinging
                // up at each end (it slows) and dropping through the middle
                acc += glide * cos(s.x) * 3.2 + vec3<f32>(0.0, 0.0, abs(sin(s.x)) * 2.2 - 1.0);
                // tilted INTO each glide, its leading edge down, level and
                // flipping over at the apex where the glide turns back
                let tilt = 0.75 * cos(s.x) - 0.35 * sin(2.0 * s.x);
                let axis = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), glide));
                let yaw = dr(seed + 16u) * 6.2831853 + 0.4 * sin(s.x * 0.5);
                q = ds_quat_mul(ds_quat_axis(axis, tilt), ds_quat_facing(vec3<f32>(0.0, 0.0, 1.0), yaw));
            } else if (mode == MODE_TUMBLE) {
                // end over end about a level axis, drifting sideways as it
                // spins (Magnus)
                let axis = normalize(cross(vec3<f32>(0.0, 0.0, 1.0), glide));
                acc += cross(axis, vel - air) * 0.6;
                q = ds_quat_mul(ds_quat_axis(axis, s.x * 2.4), ds_quat_facing(glide, 0.0));
            } else {
                // a spiral: round a small circle, banked into it
                let r = 0.25 + 0.4 * dr(seed + 17u);
                let w = 3.0 * rate;
                let round = vec3<f32>(cos(s.x * 1.4), sin(s.x * 1.4), 0.0);
                acc += -round * r * w * w * 0.5;
                q = ds_quat_mul(ds_quat_axis(vec3<f32>(-round.y, round.x, 0.0), 0.6), ds_quat_facing(vec3<f32>(0.0, 0.0, 1.0), s.x * 1.4));
            }
            // it changes how it falls now and then
            if (dr(frame_seed + 18u) < dt * 0.15) {
                let m = dr(frame_seed + 19u);
                let next = select(select(MODE_SPIRAL, MODE_TUMBLE, m < 0.7), MODE_FLUTTER, m < 0.45);
                state = (state & ~(15u << DS_MODE_SHIFT)) | (next << DS_MODE_SHIFT);
            }
        } else if (kind == DK_EMBERS) {
            // hot, it rises (more in the gusts) and flickers about; as it
            // cools its lift fades, and a dying ember drifts back down
            let heat = clamp(1.0 - s.z / k.life, 0.0, 1.0);
            acc += vec3<f32>(0.0, 0.0, (1.2 * dp.wind.z + 0.8) * heat * heat - 2.2 * (1.0 - heat));
            s.x += dt * 9.0 * rate;
        } else if (kind == DK_MOTES) {
            // dust motes: nearly as light as the air, they hang and wander
            // with its slightest stirring
            let wander = vec3<f32>(sin(s.z * 0.7 + f32(seed & 255u)), cos(s.z * 0.53 + f32((seed >> 8u) & 255u)), sin(s.z * 0.31 + f32((seed >> 16u) & 255u)) * 0.5);
            acc += wander * 0.08;
        } else {
            // sand: the turbulence near the ground kicks it up and about
            let kick = vec3<f32>(dr(frame_seed + 21u) - 0.5, dr(frame_seed + 22u) - 0.5, dr(frame_seed + 23u) - 0.5);
            acc += kick * (2.0 + 3.0 * dp.wind.z) * clamp(length(dp.wind.xy) / 6.0, 0.2, 1.5);
            s.x += dt;
        }
        vel += acc * dt;
        // quadratic drag toward the air, solved for the end of the step
        let rel = vel - air;
        vel = air + rel / (1.0 + kdrag * length(rel) * dt);
        var to = p.xyz + vel * dt;
        let h = ds_ground(to.xy, p.z);
        if (h > -1e8 && to.z <= h + 0.01) {
            if (h > p.z + 0.4) {
                // into a wall (the top it shows is well above): bounce back
                // off it, and leave the cell it would enter
                vel = vec3<f32>(-vel.xy * 0.3, vel.z);
                to = vec3<f32>(p.xy, p.z);
            } else if (kind == DK_EMBERS || kind == DK_MOTES) {
                // an ember that touches the ground goes out; a mote settles
                // out of sight
                p.w = 0.0;
            } else if (kind == DK_SAND) {
                // sand hops along the ground in the wind (saltation); in a
                // lull it settles and its slot is free again (the sand lying
                // is the drifts', not particles')
                if (length(vel.xy) > 1.5) {
                    to.z = h + 0.01;
                    vel = vec3<f32>(vel.xy * 0.7, abs(vel.z) * 0.6 + 0.4);
                } else {
                    p.w = 0.0;
                }
            } else {
                // it lands, lying on the surface with a yaw of its own
                to.z = h + 0.004;
                let n = ds_normal(to.xy, h, to.z);
                q = ds_quat_facing(n, dr(seed + 20u) * 6.2831853);
                vel = vec3<f32>(vel.xy * 0.2, 0.0);
                state |= DS_LANDED;
                s.z = 0.0;
            }
        }
        // (under a roof it does not blow: a top above it is a wall it stops
        // against; under a canopy it falls through to the floor beneath)
        p = vec4<f32>(to, p.w);
        // too old in the air: it fades (embers burn out; sand settles out)
        if (s.z > k.life) { p.w -= dt / 2.0; }
    }
    if (p.w <= 0.0) {
        debris[b] = vec4<f32>(0.0);
        return;
    }
    debris[b] = p;
    debris[b + 1u] = vec4<f32>(vel, bitcast<f32>(state));
    debris[b + 2u] = normalize(q);
    debris[b + 3u] = s;
}
