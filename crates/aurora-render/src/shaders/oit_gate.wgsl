// The OIT's gated draws as plain indirect draws (oit_gate.rs). Each gated
// draw's arguments are copied from the indirect buffer into three copies
// whose instance counts the GPU keeps or zeroes, so that drawing a copy
// draws all of a range or nothing, as a count read from the GPU would, on
// every backend (Metal has no counted indirect draws):
// - before the capture, the capture's copy: no instance while the pool's
//   hold-off after an overflow runs (the frame is then drawn in order, as
//   an overflowed one, without the capture and its lists);
// - after the classify (oit.wgsl), the replay's copy, with instances when
//   the pool overflowed or the hold-off runs, and the composite's copy,
//   with instances otherwise. One thread then moves the hold-off on.

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
const NONE: u32 = 0xffffffffu;
// The pool's flags (oit.rs, set by oit.wgsl `set_flags`): [1] the replay's
// instances, [2] whether the pool overflowed this frame, [4] the frames
// left of the hold-off, [5] the length of the last hold-off.
const REPLAY: u32 = 1u;
const OVERFLOWED: u32 = 2u;
const HOLD: u32 = 4u;
const LAST_HOLD: u32 = 5u;
// A hold-off after an overflow: this many frames at first, twice the last
// one after each probe that overflows again, up to about a second: a scene
// too deep for the pool costs its in-order drawing plus one capture a
// second, not a capture, a classify and a replay every frame.
const MIN_HOLD: u32 = 4u;
const MAX_HOLD: u32 = 128u;

@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read_write> flags: array<u32>;
@group(0) @binding(2) var<storage, read_write> gated: array<u32>;
@group(0) @binding(3) var<uniform> gate: Gate;

// The draw of the indirect buffer behind gated draw `i`, or NONE.
fn source_draw(i: u32) -> u32 {
    for (var s = 0u; s < 2u; s++) {
        let sp = gate.span[s];
        if (i >= sp.z && i < sp.z + sp.y) {
            return sp.x + (i - sp.z);
        }
    }
    return NONE;
}

// Draw `src` of the indirect buffer into gated draw `dst`, with `instances`.
fn copy_draw(src: u32, dst: u32, instances: u32) {
    for (var k = 0u; k < ARGS; k++) {
        gated[dst * ARGS + k] = source[src * ARGS + k];
    }
    gated[dst * ARGS + 1u] = instances;
}

// Before the capture: its copy (the third), without instances while the
// hold-off runs.
@compute @workgroup_size(64)
fn cs_gate_capture(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= gate.total) {
        return;
    }
    let src = source_draw(i);
    if (src == NONE) {
        return;
    }
    let instances = select(source[src * ARGS + 1u], 0u, flags[HOLD] > 0u);
    copy_draw(src, 2u * gate.total + i, instances);
}

// After the classify: the replay's copy (the first) and the composite's
// (the second).
@compute @workgroup_size(64)
fn cs_gate(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i == 0u) {
        // The hold-off for the next frame, by one thread (the others read
        // the replay flag only): a frame of it passes; or a probe that
        // overflowed starts a longer one; or a probe that fit ends them.
        let hold = flags[HOLD];
        if (hold > 0u) {
            flags[HOLD] = hold - 1u;
        } else if (flags[OVERFLOWED] != 0u) {
            let next = clamp(flags[LAST_HOLD] * 2u, MIN_HOLD, MAX_HOLD);
            flags[HOLD] = next;
            flags[LAST_HOLD] = next;
        } else {
            flags[LAST_HOLD] = 0u;
        }
    }
    if (i >= gate.total) {
        return;
    }
    let src = source_draw(i);
    if (src == NONE) {
        return;
    }
    let over = flags[REPLAY] != 0u;
    let instances = source[src * ARGS + 1u];
    copy_draw(src, i, select(0u, instances, over));
    copy_draw(src, gate.total + i, select(instances, 0u, over));
}
