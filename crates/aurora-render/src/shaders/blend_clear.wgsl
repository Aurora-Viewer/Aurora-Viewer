// Exact blend order (blend_exact.rs): empties the fronts of the pixels the
// last view filed fragments in. Emptying every word of every pixel each
// frame would cost more than drawing a scene with a few panes of glass.
// The first slots of all the pixels come first in the buffer, one word
// each, and a pixel that took a fragment has its own set: reading that one
// word tells whether the rest of the pixel was touched at all.

// `BlendOrderInfo` of object.wgsl
struct BlendOrderInfo {
    width: u32,
    height: u32,
    slots: u32,
    rest: u32,
};

@group(0) @binding(0) var<uniform> info: BlendOrderInfo;
@group(0) @binding(1) var<storage, read_write> fronts: array<u32>;

// Pixels a thread reads, side by side: their first slots are 64 bytes.
const RUN: u32 = 16u;

// A group of 8 x 8 threads covers 128 x 8 pixels (`blend_exact::CLEAR_GROUP`).
@compute @workgroup_size(8, 8)
fn cs_blend_clear(@builtin(global_invocation_id) at: vec3<u32>) {
    if (at.y >= info.height) {
        return;
    }
    let x0 = at.x * RUN;
    let x1 = min(x0 + RUN, info.width);
    for (var x = x0; x < x1; x++) {
        let pixel = at.y * info.width + x;
        if (fronts[pixel] != 0u) {
            fronts[pixel] = 0u;
            let rest = info.width * info.height + pixel * info.rest;
            for (var i = 0u; i < info.rest; i++) {
                fronts[rest + i] = 0u;
            }
        }
    }
}
