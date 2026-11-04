// The OIT's blended draws per band as plain indirect draws (oit_gate.rs).
// Once the band plan (oit.wgsl cs_band_plan) knows which bands the frame
// uses, the arguments of the blended draws are copied once per band from
// the indirect buffer, their instances kept for a band with rows and
// zeroed for the others. Drawing a band's copy then draws all of the list
// or nothing, as a count read from the GPU would, on every backend (Metal
// has no counted indirect draws).

struct Gate {
    // draws in each copy
    total: u32,
    // copies (the bands)
    bands: u32,
    // first draw in the indirect buffer
    src: u32,
    _c: u32,
}

// DrawIndexedIndirect: index count, instance count, first index, base
// vertex, first instance
const ARGS: u32 = 5u;

@group(0) @binding(0) var<storage, read> source: array<u32>;
// oit.wgsl: [1 + b] band b's rows (z, w: its tile rows; empty when equal)
@group(0) @binding(1) var<storage, read> bands: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> gated: array<u32>;
@group(0) @binding(3) var<uniform> gate: Gate;

@compute @workgroup_size(64)
fn cs_gate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= gate.total * gate.bands) {
        return;
    }
    let d = i % gate.total;
    let b = i / gate.total;
    let band = bands[1u + b];
    let src = (gate.src + d) * ARGS;
    let dst = i * ARGS;
    for (var k = 0u; k < ARGS; k++) {
        gated[dst + k] = source[src + k];
    }
    gated[dst + 1u] = select(0u, source[src + 1u], band.w > band.z);
}
