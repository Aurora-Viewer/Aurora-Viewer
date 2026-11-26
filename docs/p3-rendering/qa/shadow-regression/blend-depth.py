"""Compare actual contact and occlusion depth prepasses over detached blended texels."""

from pathlib import Path
import re
import struct
import wgpu

ALPHAS = [0.0, 0.25, 0.499, 0.5, 0.75, 0.879, 0.88, 0.95, 1.0]


def production_function(shader, name):
    start = shader.index(f"fn {name}(")
    return shader[start : shader.index("\n}", start) + 2]


def run_blend_depth(object_path, device):
    shader = Path(object_path).read_text(encoding="utf-8")
    common = Path(object_path).with_name("common.wgsl").read_text(encoding="utf-8")
    constants = ""
    for name, src in [
        ("DEPTH_CONTACT_ALPHA", shader),
        ("OCCLUSION_RISE", shader),
        ("SCREEN_BLEND_ALPHA", common),
    ]:
        constants += (
            re.search(rf"^(?:const|override) {name}: .+?;", src, re.M)[0] + "\n"
        )
    dim = (len(ALPHAS), 1, 1)
    code = (
        constants
        + """
const FLAG_DOUBLE_SIDED = 1u;
const FLAG_PORTAL_EFFECT = 2u;
const PORTAL_SURFACE_SCALE = 1.16;
struct Record { flags: vec4u, tex: vec4u, base_color: vec4f, }
const records = array(Record(vec4u(1), vec4u(0), vec4f(1)));
struct VsOut {
    @builtin(position) clip:vec4f,
    @location(0) @interpolate(flat) record:u32,
    @location(1) world_pos:vec3f,
    @location(2) uv:vec2f
}
fn portal_clip(rec: Record, world: vec3f) {}
fn water_clip(world: vec3f) {}
@group(0) @binding(0) var solid_depth: texture_depth_2d;
"""
    )
    code += f"const alpha_values = array({', '.join(str(a) for a in ALPHAS)});\n"
    code += """
fn sample_tex(slot: u32, uv: vec2f) -> vec4f {
    return vec4f(0, 0, 0, alpha_values[u32(uv.x)]);
}
@vertex fn vs(@builtin(vertex_index) id: u32) -> VsOut {
    let p = array(vec2f(-1,-1), vec2f(3,-1), vec2f(-1,3))[id];
    return VsOut(vec4f(p,0.5,1), 0u, vec3f(0), vec2f((p.x+1.0) * f32(arrayLengthPlaceholder) * 0.5, 0.0));
}
""".replace("arrayLengthPlaceholder", str(len(ALPHAS)))
    for name in [
        "depth_contact",
        "risen_depth",
        "blend_contact_covered",
        "fs_depth_contact",
        "fs_depth_occlusion",
    ]:
        if name.startswith("fs_"):
            code += "@fragment\n"
        code += production_function(shader, name) + "\n"
    code += """
@fragment fn eligible(i: VsOut) -> @builtin(frag_depth) f32 {
    return select(0.4, 0.5, blend_contact_covered(sample_tex(0u,i.uv).a, true));
}
@fragment fn prepass_only(i: VsOut) -> @builtin(frag_depth) f32 {
    return select(0.4, 0.5, blend_contact_covered(sample_tex(0u,i.uv).a, false));
}
"""
    module = device.create_shader_module(code=code)
    fmt = "depth32float"

    def make_texture(usage):
        return device.create_texture(
            size=dim, format=fmt, usage=wgpu.TextureUsage.RENDER_ATTACHMENT | usage
        )

    def begin(encoder, tex):
        return encoder.begin_render_pass(
            color_attachments=[],
            depth_stencil_attachment={
                "view": tex.create_view(),
                "depth_clear_value": 0.4,
                "depth_load_op": "clear",
                "depth_store_op": "store",
            },
        )

    solid = make_texture(wgpu.TextureUsage.TEXTURE_BINDING)
    encoder = device.create_command_encoder()
    begin(encoder, solid).end()
    device.queue.submit([encoder.finish()])
    results = {}
    for entry in ["fs_depth_contact", "fs_depth_occlusion", "eligible", "prepass_only"]:
        pipe = device.create_render_pipeline(
            layout="auto",
            vertex={"module": module, "entry_point": "vs"},
            fragment={"module": module, "entry_point": entry, "targets": []},
            depth_stencil={
                "format": fmt,
                "depth_write_enabled": True,
                "depth_compare": "greater",
            },
        )
        depth = make_texture(wgpu.TextureUsage.COPY_SRC)
        encoder = device.create_command_encoder()
        render = begin(encoder, depth)
        render.set_pipeline(pipe)
        if entry == "fs_depth_occlusion":
            group = device.create_bind_group(
                layout=pipe.get_bind_group_layout(0),
                entries=[{"binding": 0, "resource": solid.create_view()}],
            )
            render.set_bind_group(0, group)
        render.draw(3)
        render.end()
        device.queue.submit([encoder.finish()])
        raw = device.queue.read_texture(
            {"texture": depth, "aspect": "depth-only"},
            {"offset": 0, "bytes_per_row": 256, "rows_per_image": 1},
            dim,
        )
        values = struct.unpack_from(f"{len(ALPHAS)}f", raw)
        assert all(abs(z - 0.4) < 1e-5 or abs(z - 0.5) < 1e-5 for z in values), (
            entry,
            values,
        )
        results[entry] = [
            alpha for alpha, z in zip(ALPHAS, values) if abs(z - 0.5) < 1e-5
        ]
        depth.destroy()
    solid.destroy()
    assert results["fs_depth_contact"] == [a for a in ALPHAS if a >= 0.5]
    assert results["fs_depth_occlusion"] == [a for a in ALPHAS if a >= 0.88]
    assert results["eligible"] == results["fs_depth_contact"]
    assert results["prepass_only"] == []
    return results


if __name__ == "__main__":
    import argparse
    import json

    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("object_wgsl", type=Path)
    args = p.parse_args()
    print(
        json.dumps(
            run_blend_depth(
                args.object_wgsl, wgpu.gpu.request_adapter_sync().request_device_sync()
            ),
            indent=2,
        )
    )
