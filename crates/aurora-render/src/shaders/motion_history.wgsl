// The records' model matrices as this frame drew them, kept for the next
// frame's motion vectors (motion_history.rs). Only the matrix of a record is
// read: the rest of `DrawRecord` (common.wgsl, 288 bytes) is skipped.

struct Record {
    model: mat4x4<f32>,
    rest: array<vec4<f32>, 14>,
};

@group(0) @binding(0) var<storage, read> records: array<Record>;
@group(0) @binding(1) var<storage, read_write> prev_models: array<mat4x4<f32>>;

// Dispatched over the record slots handed out (rounded up to the
// workgroup: the few past them copy unused slots).
@compute @workgroup_size(64)
fn gather(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= arrayLength(&prev_models) || id.x >= arrayLength(&records)) {
        return;
    }
    prev_models[id.x] = records[id.x].model;
}
