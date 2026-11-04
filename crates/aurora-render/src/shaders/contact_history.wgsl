// Short, surface-validated shadow history. Current coverage and blocker
// distance reject moving-caster changes, even on a stationary receiver.
struct Params {
    inverse_current: mat4x4<f32>,
    previous: mat4x4<f32>,
    jitter_reset: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Params;
@group(0) @binding(1) var current: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var motion: texture_2d<f32>;
@group(0) @binding(4) var history: texture_2d<f32>;
@group(0) @binding(5) var metadata: texture_2d<u32>;
@group(0) @binding(6) var next_history: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(7) var next_metadata: texture_storage_2d<rg32uint, write>;
@group(0) @binding(8) var result: texture_storage_2d<rgba8unorm, write>;

fn normal_from_oct(e: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(e, 1.0 - abs(e.x) - abs(e.y));
    if (n.z < 0.0) { n = vec3<f32>((1.0 - abs(n.yx)) * sign(n.xy), n.z); }
    return normalize(n);
}

// A disappearing caster must clear immediately. A changing blocker cannot
// lend its history to a new one; small residuals get at most four-frame weight.
fn history_weight(cur: vec2<f32>, old: vec2<f32>, movement: f32) -> f32 {
    if (abs(cur.x - old.x) > 0.2 || abs(cur.y - old.y) > 2.0 / 255.0) { return 0.0; }
    if ((cur.x <= 1.0 / 255.0) != (old.x <= 1.0 / 255.0)) { return 0.0; }
    return mix(0.75, 0.5, smoothstep(0.25, 2.0, movement));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(current));
    let p = vec2<i32>(id.xy);
    if (any(p >= size)) { return; }
    let cur = textureLoad(current, p, 0);
    let d = textureLoad(depth, p, 0);
    let n = textureLoad(motion, p, 0);
    var value = cur;
    let uv = (vec2<f32>(p) + 0.5) / vec2<f32>(size);
    let world = u.inverse_current * vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), d, 1.0);
    let prev = u.previous * world;
    if (u.jitter_reset.z < 0.5 && d > 0.0 && prev.w > 0.0) {
        let expected_depth = prev.z / prev.w;
        let cam_uv = prev.xy / prev.w * vec2<f32>(0.5, -0.5) + 0.5;
        let prev_uv = uv - n.zw + u.jitter_reset.xy;
        let own_motion = length((prev_uv - cam_uv) * vec2<f32>(size));
        let at = prev_uv * vec2<f32>(size) - 0.5;
        let base = vec2<i32>(floor(at));
        let fraction = fract(at);
        var sum = 0.0;
        var weight = 0.0;
        let current_normal = normal_from_oct(n.xy);
        for (var k = 0; k < 4; k++) {
            let offset = vec2<i32>(k & 1, k >> 1);
            let q = base + offset;
            if (any(q < vec2<i32>(0)) || any(q >= size)) { continue; }
            let previous_meta = textureLoad(metadata, q, 0).xy;
            let old_depth = bitcast<f32>(previous_meta.x);
            let old_normal = normal_from_oct(unpack2x16float(previous_meta.y));
            // Independent object motion has no previous depth component in
            // the UV velocity buffer. Reject ambiguous depth instead of
            // accepting a wide interval that could include another surface.
            let tolerance = max(1e-6, abs(expected_depth) * 0.01);
            if (old_depth <= 0.0 || abs(old_depth - expected_depth) > tolerance
                || dot(current_normal, old_normal) < 0.9) { continue; }
            let old = textureLoad(history, q, 0);
            let valid = history_weight(cur.rg, old.rg, max(own_motion, length(n.zw * vec2<f32>(size))));
            let bilinear = select(1.0 - fraction.x, fraction.x, offset.x == 1)
                * select(1.0 - fraction.y, fraction.y, offset.y == 1);
            sum += bilinear * valid * clamp(old.r, cur.r - 0.125, cur.r + 0.125);
            weight += bilinear * valid;
        }
        value.r = cur.r * (1.0 - weight) + sum;
    }
    textureStore(result, p, value);
    textureStore(next_history, p, value);
    textureStore(next_metadata, p, vec4<u32>(bitcast<u32>(d), pack2x16float(n.xy), 0u, 0u));
}
