"""GPU readback of the production P3 layered-shadow lookup. Requires wgpu-py 0.33.

python remote-coverage.py path/to/crates/aurora-render/src/shaders/common.wgsl
Pass --measure to print another revision's results without enforcing the repair.
"""

import argparse
import json
import math
import struct
from pathlib import Path

import wgpu


def render_remote_depth(device):
    texture = device.create_texture(
        size=(128, 128, 1),
        mip_level_count=2,
        format="depth32float",
        usage=wgpu.TextureUsage.RENDER_ATTACHMENT | wgpu.TextureUsage.TEXTURE_BINDING,
    )
    module = device.create_shader_module(
        code="""
        @vertex fn vs(@builtin(vertex_index) vertex: u32) -> @builtin(position) vec4<f32> {
            let corners = array<vec2<f32>, 3>(vec2(-1., -1.), vec2(3., -1.), vec2(-1., 3.));
            return vec4(corners[vertex], 0., 1.);
        }
        @fragment fn fs(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
            // Half plane for the edge and duplicated-slab tests; isolated
            // one-texel-wide remote sliver for the soft penumbra's blocker-search test.
            let blocked = (p.x < 16. && p.y < 12.) ||
                (u32(p.x) == 15u && u32(p.y) >= 21u && u32(p.y) <= 23u);
            return select(1., 0.3, blocked);
        }
    """
    )
    pipeline = device.create_render_pipeline(
        layout="auto",
        vertex={"module": module, "entry_point": "vs"},
        fragment={"module": module, "entry_point": "fs", "targets": []},
        depth_stencil={
            "format": "depth32float",
            "depth_write_enabled": True,
            "depth_compare": "always",
        },
    )
    commands = device.create_command_encoder()
    for level in range(2):
        layer = texture.create_view(base_mip_level=level, mip_level_count=1)
        render = commands.begin_render_pass(
            color_attachments=[],
            depth_stencil_attachment={
                "view": layer,
                "depth_clear_value": 1,
                "depth_load_op": "clear",
                "depth_store_op": "store",
            },
        )
        if level == 1:
            render.set_pipeline(pipeline)
            render.draw(3)
        render.end()
    device.queue.submit([commands.finish()])
    return texture


STOCK_TANGENT = math.tan(math.radians(0.53 / 2))
# Integral of the negative half of the x coordinate of a unit disk.
MEAN_NEGATIVE_X = -2.0 / (3.0 * math.pi)
QUERIES = [
    ("hard_half_plane_blocked_side", 15.999, 8.5, 0.0, 1.0, 0.499),
    ("hard_half_plane_lit_side", 16.001, 8.5, 0.0, 1.0, 0.501),
    ("hard_duplicate_slabs", 15.999, 8.5, 0.0, 0.499, 0.499),
    ("soft_isolated_left", 15.999, 22.5, STOCK_TANGENT, 1.0, 0.499),
    ("soft_isolated_right", 16.001, 22.5, STOCK_TANGENT, 1.0, 0.501),
    ("soft_duplicate_slabs", 15.999, 8.5, 0.001, 0.499, 0.499 + 0.02 * MEAN_NEGATIVE_X),
    ("hard_lit", 26.5, 22.5, 0.0, 1.0, 1.0),
    ("hard_blocked", 1.5, 8.5, 0.0, 1.0, 0.0),
    ("soft_lit", 26.5, 22.5, 0.001, 1.0, 1.0),
    ("local_core", 16.0, 8.5, 0.001, 0.0, 0.0),
]


def measure(shader):
    adapter = wgpu.gpu.request_adapter_sync()
    device = adapter.request_device_sync()
    depths = render_remote_depth(device)
    storage = device.create_buffer(
        size=4 * len(QUERIES),
        usage=wgpu.BufferUsage.STORAGE | wgpu.BufferUsage.COPY_SRC,
    )
    begin = shader.index("const PCSS_MAX_METRES:")
    end = shader.index("fn pcss_compose(", begin)
    test_source = (
        """
        struct Frame { detail_params: vec4<f32> };
        var<private> frame: Frame;
        @group(0) @binding(0) var shadow_map: texture_depth_2d;
        @group(0) @binding(1) var shadow_sampler: sampler_comparison;
        @group(0) @binding(2) var<storage, read_write> output: array<f32>;
    """
        + shader[begin:end]
        + """
        @compute @workgroup_size(1) fn test(@builtin(global_invocation_id) id: vec3<u32>) {
            let inputs = array("""
    )
    test_source += ", ".join(
        "vec4<f32>(%s, %s, %s, %s)" % (x, y, angle, local)
        for _, x, y, angle, local, _ in QUERIES
    )
    test_source += """);
            let q = inputs[id.x];
            frame.detail_params.w = q.z;
            let vp = mat4x4<f32>(vec4(1., 0., 0., 0.), vec4(0., 1., 0., 0.),
                                 vec4(0., 0., 1., 0.), vec4(0., 0., 0., 1.));
            let uv = q.xy / 32.;
            output[id.x] = pcss_compose_grids(shadow_map, q.w, vec3(0.), vp, vec3(0., 0., 1.),
                uv, 0.7, 0., vec2(0.), 0u, 64., 0.01, 1., uv, 64., 0.01, 0.);
        }
    """
    module = device.create_shader_module(code=test_source)
    pipeline = device.create_compute_pipeline(
        layout="auto", compute={"module": module, "entry_point": "test"}
    )
    sampler = device.create_sampler(
        compare="less-equal", min_filter="linear", mag_filter="linear"
    )
    group = device.create_bind_group(
        layout=pipeline.get_bind_group_layout(0),
        entries=[
            {"binding": 0, "resource": depths.create_view()},
            {"binding": 1, "resource": sampler},
            {"binding": 2, "resource": {"buffer": storage}},
        ],
    )
    encoder = device.create_command_encoder()
    compute = encoder.begin_compute_pass()
    compute.set_pipeline(pipeline)
    compute.set_bind_group(0, group)
    compute.dispatch_workgroups(len(QUERIES))
    compute.end()
    device.queue.submit([encoder.finish()])
    values = struct.unpack(f"<{len(QUERIES)}f", device.queue.read_buffer(storage))
    return adapter.info["device"], values


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shader", type=Path)
    parser.add_argument("--measure", action="store_true")
    opts = parser.parse_args()
    name, values = measure(opts.shader.read_text(encoding="utf-8"))
    print(
        json.dumps(
            {"adapter": name, "visibility": {q[0]: v for q, v in zip(QUERIES, values)}},
            indent=2,
        )
    )
    if not opts.measure:
        for q, v in zip(QUERIES, values):
            assert abs(v - q[-1]) < 0.001, (q[0], v, q[-1])
