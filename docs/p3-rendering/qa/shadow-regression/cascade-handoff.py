"""Rasterize one 16 mm post in two sun cascades and scan a sloping receiver.

Uses the real shadow_factor, cascade lookups and filter from the supplied WGSL.
Requires wgpu-py 0.33. Run with the path to common.wgsl. Add --measure when
recording the pre-fix shader, which is expected to fail the single-post test.
"""

import argparse
import json
import re
import struct
from pathlib import Path

import wgpu


TILE = 1024
COUNT = 161
START_MM = -80
STEP_MM = 0.5
DEPTHS = [1.65, 1.775, 1.85, 1.925, 2.0, 2.03]


def make_atlas(device):
    atlas = device.create_texture(
        size=(TILE * 2, TILE * 2, 1),
        format="depth32float",
        usage=wgpu.TextureUsage.RENDER_ATTACHMENT | wgpu.TextureUsage.TEXTURE_BINDING,
    )
    vertex = device.create_shader_module(
        code="""
        @vertex fn vs(@builtin(vertex_index) id:u32, @builtin(instance_index) tile:u32) -> @builtin(position) vec4<f32> {
            let vertices = array<vec2<f32>,6>(vec2(-1.,-1.),vec2(1.,-1.),vec2(-1.,1.),
                                                 vec2(-1.,1.),vec2(1.,-1.),vec2(1.,1.));
            let half_extent = select(0.512,1.024,tile == 1u);
            let xy = vertices[id] * vec2(0.008,0.20);
            return vec4(xy / half_extent,0.30,1.);
        }
    """
    )
    pipeline = device.create_render_pipeline(
        layout="auto",
        vertex={"module": vertex, "entry_point": "vs"},
        depth_stencil={
            "format": "depth32float",
            "depth_write_enabled": True,
            "depth_compare": "less",
        },
    )
    encoder = device.create_command_encoder()
    render = encoder.begin_render_pass(
        color_attachments=[],
        depth_stencil_attachment={
            "view": atlas.create_view(),
            "depth_clear_value": 1.0,
            "depth_load_op": "clear",
            "depth_store_op": "store",
        },
    )
    render.set_pipeline(pipeline)
    for tile in range(2):
        render.set_viewport(tile * TILE, 0, TILE, TILE, 0, 1)
        render.set_scissor_rect(tile * TILE, 0, TILE, TILE)
        render.draw(6, 1, 0, tile)
    render.end()
    device.queue.submit([encoder.finish()])
    return atlas


def production_test_shader(common):
    start = common.index("fn shadow_taps(")
    end = common.index("// Four-level directional shadow sampling", start)
    factor = "\n".join(re.search(r"^fn " + fn + r"\(.*?^\}", common, re.S | re.M).group()
                       for fn in ("shadow_factor", "mapped_shadow", "cascade_visibility"))
    constants = "\n".join(
        re.search(r"^const " + name + r":.*?;", common, re.M).group()
        for name in [
            "SHADOW_PENUMBRA",
            "SHADOW_FILTER_GAPS",
            "SHADOW_BIAS_LEGACY",
            "RC_FLAG_NEAR_SHADOW_DETAIL",
        ]
    )
    return (
        """
        struct Frame {
            detail_params:vec4<f32>, cascade_splits:vec4<f32>, cascade_vp:array<mat4x4<f32>,3>,
            params:vec4<f32>, render_controls:vec4<u32>, light_mode:vec4<u32>, clip_plane:vec4<f32>,camera_pos:vec4<f32>
        };
        var<private> frame:Frame;
        @group(0) @binding(0) var shadow_map:texture_depth_2d;
        @group(0) @binding(1) var shadow_sampler:sampler_comparison;
        @group(0) @binding(2) var<storage,read_write> out:array<f32>;
        // This fixture selects the main-view, three-level path.
        fn in_reflection_pass()->bool {return false;}
        fn graded_distance(p:vec3<f32>)->f32 {return 0.;}
        fn graded_shadow(p:vec3<f32>,n:vec3<f32>,d:f32)->vec4<f32> {return vec4(1.);}
    """
        + constants
        + common[start:end]
        + factor
        + """
        fn projection(half_extent:f32)->mat4x4<f32> {
            return mat4x4<f32>(vec4(1./half_extent,0.,0.,0.),vec4(0.,1./half_extent,0.,0.),
                               vec4(0.,0.,1.,0.),vec4(0.,0.,0.,1.));
        }
        @compute @workgroup_size(1) fn test(@builtin(global_invocation_id) id:vec3<u32>) {
            frame.cascade_vp[0]=projection(0.512); frame.cascade_vp[1]=projection(1.024);
            frame.cascade_vp[2]=projection(2.048); frame.params.z=1.;
            frame.cascade_splits=vec4(2.,5.,12.,0.);
            let depths=array<f32,6>(1.65,1.775,1.85,1.925,2.0,2.03);
            let x=-0.080+0.0005*f32(id.x);
            // z falls with x, so (0.8,0,0.6) is the slope's actual normal.
            let p=vec3(x,0.,0.7-(0.8/0.6)*x);
            out[id.y*161u+id.x]=shadow_factor(p,vec3(0.8,0.,0.6),depths[id.y]);
        }
    """
    )


def main(common):
    adapter = wgpu.gpu.request_adapter_sync()
    device = adapter.request_device_sync()
    atlas = make_atlas(device)
    storage = device.create_buffer(
        size=4 * COUNT * len(DEPTHS),
        usage=wgpu.BufferUsage.STORAGE | wgpu.BufferUsage.COPY_SRC,
    )
    module = device.create_shader_module(code=production_test_shader(common))
    pipeline = device.create_compute_pipeline(
        layout="auto", compute={"module": module, "entry_point": "test"}
    )
    sampler = device.create_sampler(
        compare="less-equal", min_filter="linear", mag_filter="linear"
    )
    group = device.create_bind_group(
        layout=pipeline.get_bind_group_layout(0),
        entries=[
            {"binding": 0, "resource": atlas.create_view()},
            {"binding": 1, "resource": sampler},
            {"binding": 2, "resource": {"buffer": storage}},
        ],
    )
    encoder = device.create_command_encoder()
    compute = encoder.begin_compute_pass()
    compute.set_pipeline(pipeline)
    compute.set_bind_group(0, group)
    compute.dispatch_workgroups(COUNT, len(DEPTHS))
    compute.end()
    device.queue.submit([encoder.finish()])
    values = struct.unpack(
        f"<{COUNT * len(DEPTHS)}f", device.queue.read_buffer(storage)
    )
    summary = []
    for i, depth in enumerate(DEPTHS):
        row = values[i * COUNT : (i + 1) * COUNT]
        runs = []
        indices = []
        for j, v in enumerate((*row, 1.0)):
            if v < 0.8:
                indices.append(j)
            elif indices:
                runs.append(
                    [START_MM + STEP_MM * indices[0], START_MM + STEP_MM * indices[-1]]
                )
                indices = []
        summary.append(
            {
                "receiver_depth_m": depth,
                "bands_mm": runs,
                "min_visibility": min(row),
                "visibility_at_handoff_center": row[96],
            }
        )
    return {"adapter": adapter.info["device"], "post_width_mm": 16, "samples": summary}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("shader", type=Path)
    parser.add_argument("--measure", action="store_true")
    opts = parser.parse_args()
    measurements = main(opts.shader.read_text(encoding="utf-8"))
    print(json.dumps(measurements, indent=2))
    if not opts.measure:
        rows = measurements["samples"]
        assert all(len(r["bands_mm"]) == 1 for r in rows)
        assert rows[2]["visibility_at_handoff_center"] < 0.1
        assert all(r["min_visibility"] < 0.1 for r in rows)
