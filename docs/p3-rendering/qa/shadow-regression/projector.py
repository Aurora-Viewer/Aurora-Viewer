"""Rasterize a one-texel caster and scan it at eighth-texel intervals.

The tested functions are the selected viewer's production WGSL. The reference
is the five weighted PCF comparisons of original P3 v2.4.3 on a stripe.
"""

import struct
from pathlib import Path

import wgpu

SAMPLES = 96
HEADER = """
struct Projector { tex: vec4<u32>, }
struct Frame {
    spot_vp: array<mat4x4<f32>, 1>,
    projectors: array<Projector, 1>,
    soft: vec4<f32>,
    params: vec4<f32>,
    cascade_splits: vec4<f32>,
    camera_pos: vec4<f32>,
    spot_fade: array<vec4<f32>, 1>,
}
var<private> frame: Frame;
@group(0) @binding(0) var shadow_map: texture_depth_2d;
@group(0) @binding(1) var shadow_sampler: sampler_comparison;
@group(0) @binding(2) var<storage,read_write> output: array<f32>;
"""

DEPTH = """
@vertex fn vs(@builtin(vertex_index) i:u32) -> @builtin(position) vec4f {
    let p = array(vec2f(-1,-1), vec2f(3,-1), vec2f(-1,3));
    return vec4f(p[i],0,1);
}
@fragment fn fs(@builtin(position) p:vec4f) -> @builtin(frag_depth) f32 {
    return select(1.0, 0.8, u32(p.x) == 640u && p.y > 600.0 && p.y < 680.0);
}
"""

SCAN = """
@compute @workgroup_size(16) fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let tile = f32(textureDimensions(shadow_map).x) * 0.25;
    frame.spot_vp[0] = mat4x4<f32>(vec4f(1,0,0,0),vec4f(0,1,0,0),vec4f(0,0,1,1),vec4f(0,0,-0.1,0));
    frame.projectors[0].tex.y = 1u;
    frame.soft.z = 0.004625;
    frame.params.z = 1.0;
    frame.cascade_splits.z = 100.0;
    let offset = (f32(gid.x) - 48.0) / 8.0;
    let point = vec3f(2.0 * (offset + 0.5) / tile, 0.0, 0.96);
    output[gid.x] = projector_shadow(0, point, vec3f(0,0,1));
}
"""


def _scan(device, source, tex, sampler, soft):
    const = f"const SOFT_PATH: bool = {str(soft).lower()};\n"
    shader = device.create_shader_module(code=HEADER + const + source + SCAN)
    pipe = device.create_compute_pipeline(
        layout="auto", compute={"module": shader, "entry_point": "main"}
    )
    buf = device.create_buffer(
        size=4 * SAMPLES, usage=wgpu.BufferUsage.COPY_SRC | wgpu.BufferUsage.STORAGE
    )
    group = device.create_bind_group(
        layout=pipe.get_bind_group_layout(0),
        entries=[
            {"binding": 0, "resource": tex.create_view()},
            {"binding": 1, "resource": sampler},
            {"binding": 2, "resource": {"buffer": buf}},
        ],
    )
    encoder = device.create_command_encoder()
    compute = encoder.begin_compute_pass()
    compute.set_pipeline(pipe)
    compute.set_bind_group(0, group)
    compute.dispatch_workgroups(SAMPLES // 16)
    compute.end()
    device.queue.submit([encoder.finish()])
    samples = list(struct.unpack(f"{SAMPLES}f", device.queue.read_buffer(buf)))
    buf.destroy()
    return samples


def run_projector(common_path, device):
    """Exercise the fixed and opt-in soft shader variants in a synthetic spotlight."""
    common = Path(common_path).read_text(encoding="utf-8")
    begin = common.index("const SOFT_DISK:")
    end = common.index("// What a local light sends", begin)
    tex = device.create_texture(
        size=(1024, 1024, 1),
        format="depth32float",
        usage=wgpu.TextureUsage.RENDER_ATTACHMENT | wgpu.TextureUsage.TEXTURE_BINDING,
    )
    shader = device.create_shader_module(code=DEPTH)
    fill = device.create_render_pipeline(
        layout="auto",
        vertex={"module": shader, "entry_point": "vs"},
        fragment={"module": shader, "entry_point": "fs", "targets": []},
        depth_stencil={
            "format": "depth32float",
            "depth_write_enabled": True,
            "depth_compare": "always",
        },
    )
    encoder = device.create_command_encoder()
    render = encoder.begin_render_pass(
        color_attachments=[],
        depth_stencil_attachment={
            "view": tex.create_view(),
            "depth_clear_value": 1,
            "depth_load_op": "clear",
            "depth_store_op": "store",
        },
    )
    render.set_pipeline(fill)
    render.draw(3)
    render.end()
    device.queue.submit([encoder.finish()])
    sampler = device.create_sampler(
        compare="less-equal", min_filter="linear", mag_filter="linear"
    )
    fixed, soft = (
        _scan(device, common[begin:end], tex, sampler, enabled)
        for enabled in (False, True)
    )
    tex.destroy()
    # The stripe covers a whole texel, so bilinear shadow comparison gives
    # triangular coverage. The five P3 taps have x offsets 0, +2, +1, -1, -2.
    reference = []
    for i in range(SAMPLES):
        x = (i - 48) / 8
        covered = sum(max(0.0, 1.0 - abs(x + off)) for off in [0, 2, 1, -1, -2]) / 5
        reference.append(1 - covered)
    max_error = max(abs(v - r) for v, r in zip(fixed, reference))
    assert max_error < 1e-5, (
        f"Fixed projector PCF differs from the P3 reference: {max_error}"
    )
    # With the centre probe within 1 texel the soft filter can find the
    # blocker directly. At the flanks it misses it and needs the PCF fallback.
    flanks = [i for i in range(SAMPLES) if 0.5 < abs((i - 48) / 8) < 3]
    # Compare lost coverage only: successful blocker probes can legitimately
    # yield a different partial penumbra from the fixed kernel.
    missing = [i for i in flanks if abs(soft[i] - 1.0) < 1e-6 and fixed[i] < 0.99]
    assert not missing, (
        f"Opt-in PCSS erased the fixed-filter edge at scan positions {missing}"
    )
    return {
        "samples": SAMPLES,
        "stock_max_error": max_error,
        "soft_flank_holes": len(missing),
    }


if __name__ == "__main__":
    import argparse
    import json

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("common_wgsl", type=Path)
    path = parser.parse_args().common_wgsl
    device = wgpu.gpu.request_adapter_sync().request_device_sync()
    print(json.dumps(run_projector(path, device), indent=2))
