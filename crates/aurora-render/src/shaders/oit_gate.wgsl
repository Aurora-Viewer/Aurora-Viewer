// The OIT's gated draws as plain indirect draws (oit_gate.rs). After the
// classify (oit.wgsl) knows whether the node pool overflowed, each gated
// draw's arguments are copied twice from the indirect buffer: the replay's
// copy keeps its instances only when the pool overflowed, the composite's
// copy only when it did not. Drawing a copy then draws all of a range or
// nothing, as a count read from the GPU would, on every backend (Metal has
// no counted indirect draws).

struct Gate {
    // gated draws in each copy
    total: u32,
    _a: u32,
    _b: u32,
    _c: u32,
    // x: first draw in the indirect buffer, y: draws, z: first gated draw
    span: array<vec4<u32>, 2>,
}

// DrawIndexedIndirect: index count, instance count, first index, base
// vertex, first instance
const ARGS: u32 = 5u;

@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read> flags: array<u32>;
@group(0) @binding(2) var<storage, read_write> gated: array<u32>;
@group(0) @binding(3) var<uniform> gate: Gate;

@compute @workgroup_size(64)
fn cs_gate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= gate.total) {
        return;
    }
    var src = 0xffffffffu;
    for (var s = 0u; s < 2u; s++) {
        let sp = gate.span[s];
        if (i >= sp.z && i < sp.z + sp.y) {
            src = sp.x + (i - sp.z);
        }
    }
    if (src == 0xffffffffu) {
        return;
    }
    // the replay depth copy's instance count (oit.wgsl): 1 when the pool
    // overflowed
    let over = flags[1] != 0u;
    let replay = i * ARGS;
    let composited = (gate.total + i) * ARGS;
    for (var k = 0u; k < ARGS; k++) {
        let v = source[src * ARGS + k];
        gated[replay + k] = v;
        gated[composited + k] = v;
    }
    let instances = source[src * ARGS + 1u];
    gated[replay + 1u] = select(0u, instances, over);
    gated[composited + 1u] = select(instances, 0u, over);
}
