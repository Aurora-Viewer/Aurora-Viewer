// The posed faces of the ray-traced shadows (rt_shadows.rs): rigged and worn
// faces have their world positions only in the vertex shaders, so each
// frame this pass writes them, one vertex per index (unindexed triangles),
// for their bottom levels. The placement is object.wgsl's `placed_model`
// (palette times inverse bind, blended over four joints; a worn prim placed
// by its anchor), without the live morphs.

struct Face {
    // The face's draw record, its first vertex and index in the arena,
    // its index count, and where its vertices start in `posed`.
    record: u32,
    base_vertex: u32,
    first_index: u32,
    index_count: u32,
    out_offset: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct Params {
    // x: faces, y: indices in all, z: vec4s of a draw record, w: threads a
    // row of workgroups covers.
    counts: vec4<u32>,
};

struct SkinBind {
    inverse_bind: mat4x4<f32>,
    joint: vec4<u32>,
};

const FLAG_SKINNED: u32 = 128u;
const FLAG_ANCHORED: u32 = 4096u;
const SKELETON_ANCHOR: u32 = 208u;
// vec4s before a record's model, its flags
const RECORD_FLAGS: u32 = 9u;
// u32 words of an arena vertex and of a skin vertex (types.rs)
const VERTEX_WORDS: u32 = 6u;
const SKIN_WORDS: u32 = 3u;

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> faces: array<Face>;
@group(0) @binding(2) var<storage, read> records: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> palettes: array<mat4x4<f32>>;
@group(0) @binding(4) var<storage, read> skin_binds: array<SkinBind>;
@group(0) @binding(5) var<storage, read> vertices: array<u32>;
@group(0) @binding(6) var<storage, read> skin: array<u32>;
@group(0) @binding(7) var<storage, read> indices: array<u32>;
@group(0) @binding(8) var<storage, read_write> posed: array<vec4<f32>>;

fn record_matrix(base: u32) -> mat4x4<f32> {
    return mat4x4<f32>(
        bitcast<vec4<f32>>(records[base]),
        bitcast<vec4<f32>>(records[base + 1u]),
        bitcast<vec4<f32>>(records[base + 2u]),
        bitcast<vec4<f32>>(records[base + 3u]),
    );
}

fn joint_matrix(flags: vec4<u32>, j: u32) -> mat4x4<f32> {
    let b = skin_binds[flags.z + j];
    return palettes[flags.y + b.joint.x] * b.inverse_bind;
}

fn index_at(i: u32) -> u32 {
    let word = indices[i >> 1u];
    return select(word & 0xffffu, word >> 16u, (i & 1u) == 1u);
}

@compute @workgroup_size(64)
fn cs_pose(@builtin(global_invocation_id) id: vec3<u32>) {
    let n = id.x + id.y * params.counts.w;
    if (n >= params.counts.y) {
        return;
    }
    // the face whose range holds `n`
    var lo = 0u;
    var hi = params.counts.x;
    while (hi - lo > 1u) {
        let mid = (lo + hi) / 2u;
        if (faces[mid].out_offset <= n) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let face = faces[lo];
    let local = n - face.out_offset;
    let v = face.base_vertex + index_at(face.first_index + local);
    let w = v * VERTEX_WORDS;
    let pos = vec3<f32>(bitcast<f32>(vertices[w]), bitcast<f32>(vertices[w + 1u]), bitcast<f32>(vertices[w + 2u]));
    let base = face.record * params.counts.z;
    let flags = records[base + RECORD_FLAGS];
    var m: mat4x4<f32>;
    if ((flags.x & FLAG_SKINNED) != 0u) {
        let s = v * SKIN_WORDS;
        let jw = skin[s];
        let joints = vec4<u32>(jw & 0xffu, (jw >> 8u) & 0xffu, (jw >> 16u) & 0xffu, jw >> 24u);
        let weights = vec4<f32>(
            f32(skin[s + 1u] & 0xffffu),
            f32(skin[s + 1u] >> 16u),
            f32(skin[s + 2u] & 0xffffu),
            f32(skin[s + 2u] >> 16u),
        ) / 65535.0;
        let ws = max(weights.x + weights.y + weights.z + weights.w, 1e-4);
        var sm = joint_matrix(flags, joints.x) * weights.x;
        if (weights.y > 0.0) { sm += joint_matrix(flags, joints.y) * weights.y; }
        if (weights.z > 0.0) { sm += joint_matrix(flags, joints.z) * weights.z; }
        if (weights.w > 0.0) { sm += joint_matrix(flags, joints.w) * weights.w; }
        sm = sm * (1.0 / ws);
        if ((flags.x & FLAG_ANCHORED) != 0u) {
            m = palettes[flags.y + SKELETON_ANCHOR] * sm;
        } else {
            m = record_matrix(base) * sm;
        }
    } else if ((flags.x & FLAG_ANCHORED) != 0u) {
        m = palettes[flags.y] * record_matrix(base);
    } else {
        m = record_matrix(base);
    }
    let world = m * vec4<f32>(pos, 1.0);
    posed[n] = vec4<f32>(world.xyz / world.w, 1.0);
}
