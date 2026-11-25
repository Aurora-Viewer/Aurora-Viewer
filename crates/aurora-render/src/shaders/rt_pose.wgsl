// The posed faces of the ray-traced shadows (rt_shadows.rs): rigged and worn
// faces have their world positions only in the vertex shaders, so on the
// frames a skeleton's bottom level is built again this pass writes them,
// one vertex per index (unindexed triangles), placed as rt_common.wgsl's
// `placed_matrix` places them. After rt_common.wgsl (rt_shadows.rs `source`).

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
    // x: faces, y: indices in all, z: unused, w: threads a row of
    // workgroups covers.
    counts: vec4<u32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> faces: array<Face>;
@group(0) @binding(2) var<storage, read_write> posed: array<vec4<f32>>;

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
    let v = face.base_vertex + index_at(face.first_index + n - face.out_offset);
    let world = placed_matrix(face.record, v) * vec4<f32>(vertex_pos(v), 1.0);
    posed[n] = vec4<f32>(world.xyz / world.w, 1.0);
}
