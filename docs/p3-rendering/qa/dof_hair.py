"""Headless depth-of-field optical regression. Run with numpy and wgpu-py 0.33.

Pass an Aurora source checkout. For the alpha-depth input, replay the optical
contract of either the AO-based before revision or the fixed renderer. Its Rust
routing test separately checks which pass feeds the lens in the actual frame.
This fixture compares the same input colours in both runs. FXAA/SMAA are
downstream of the lens. The TAA case uses the real first-frame TAA pass.
FSR colour reconstruction is held constant to isolate its display-depth pass.
"""

import argparse
import json
import re
from pathlib import Path

import numpy as np
import wgpu

B = wgpu.BufferUsage
T = wgpu.TextureUsage
F = wgpu.ShaderStage.FRAGMENT
DISPLAY = (193, 129)
HDR = "rgba16float"
DEPTH = "depth32float"

# WGSL records and the material texture are fixture inputs. The alpha test,
# AO's rising-depth rule, FSR display remap and all five DoF passes are loaded
# from the supplied checkout.
CARD_INPUTS = """
struct Record { flags: vec4u, tex: vec4u, base_color: vec4f }
const records = array(Record(vec4u(2,0,0,0), vec4u(0), vec4f(1)));
const FLAG_DOUBLE_SIDED = 2u;
const FLAG_PORTAL_EFFECT = 4u;
const PORTAL_SURFACE_SCALE = 1.0;
const OCCLUSION_RISE = 0.01;
override DEPTH_CONTACT_ALPHA: f32 = 0.5;
struct VsOut {
    @builtin(position) clip: vec4f, @location(0) @interpolate(flat) record: u32,
    @location(1) world_pos: vec3f, @location(2) uv: vec2f
}
@group(0) @binding(0) var hair: texture_2d<f32>;
@group(0) @binding(1) var solid_depth: texture_depth_2d;
fn sample_tex(slot: u32, uv: vec2f) -> vec4f {
    let hi = vec2i(textureDimensions(hair)) - 1;
    let p = clamp(vec2i(uv * vec2f(textureDimensions(hair))), vec2i(0), hi);
    return vec4f(1,1,1,textureLoad(hair, p, 0).r);
}
fn portal_clip(rec: Record, world: vec3f) {}
fn water_clip(world: vec3f) {}
fn vertex(i: u32) -> VsOut {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return VsOut(vec4f(uv * 2.0 - 1.0, 0.5, 1), 0u, vec3f(0), vec2f(uv.x,1.0-uv.y));
}
@vertex fn vs(@builtin(vertex_index) i: u32) -> VsOut { return vertex(i); }
@vertex fn vs_back(@builtin(vertex_index) i: u32) -> VsOut {
    return vertex(array(0u,2u,1u)[i]);
}
@fragment fn background(in: VsOut) -> @builtin(frag_depth) f32 {
    return select(0.05, 0.60, in.uv.x < 0.08);
}
@fragment fn color(in: VsOut) -> @location(0) vec4f {
    let alpha = sample_tex(0u, in.uv).a;
    let xy = vec2u(in.clip.xy);
    let checker = select(0.12,0.88, ((xy.x / 2u + xy.y / 2u) % 2u) == 0u);
    let sky = vec3f(checker * 0.75, checker * 0.88, checker);
    let strand = vec3f(0.055,0.032,0.018);
    return vec4f(select(mix(sky,strand,alpha),vec3f(0.78,0.18,0.05),in.uv.x < 0.08),1);
}
"""


def function(text, name):
    a = text.index(f"fn {name}(")
    return text[a : text.index("\n}", a) + 2]


def check_rgba(device, texture):
    width, height, _ = texture.size
    stride = (width * 8 + 255) // 256 * 256
    data = device.queue.read_texture({"texture": texture}, {"bytes_per_row": stride}, texture.size)
    return np.ndarray((height, width, 4), dtype=np.float16, buffer=data, strides=(stride, 8, 2)).astype(np.float32)


def check_depth(device, texture):
    width, height, _ = texture.size
    stride = (width * 4 + 255) // 256 * 256
    data = device.queue.read_texture(
        {"texture": texture, "aspect": "depth-only"}, {"bytes_per_row": stride}, texture.size
    )
    return np.ndarray((height, width), dtype=np.float32, buffer=data, strides=(stride, 4)).copy()


class Fixture:
    def __init__(self, device, root, previous=False):
        self.d = device
        self.shaders = root / "crates/aurora-render/src/shaders"
        scene = (self.shaders / "object.wgsl").read_text()
        common = (self.shaders / "common.wgsl").read_text()
        render = (self.shaders.parent / "renderer.rs").read_text()
        self.fsr_cutoff = float(re.search(r"const FSR_BLEND_ALPHA: f64 = ([.\d]+);", render)[1])
        assert self.fsr_cutoff == 0.10
        screen = re.search(r"const SCREEN_BLEND_ALPHA: f32 = [^;]+;", common)[0]
        card_shader = (
            CARD_INPUTS + screen + "\n" + function(scene, "depth_contact") + "\n" + function(scene, "risen_depth")
        )
        for name in ["fs_depth_contact", "fs_depth_occlusion"]:
            card_shader += "\n@fragment\n" + function(scene, name)
        self.card = device.create_shader_module(code=card_shader)
        self.card_source = card_shader
        self.taa = device.create_shader_module(code=(self.shaders / "taa.wgsl").read_text())
        # Select the scene fragment entry point and cutoff actually declared by
        # this checkout's new draw. Its Rust routing test covers call ordering.
        if previous:
            self.focus_fs = None
        else:
            descriptor = re.search(r"let pre_focus = sided_pipe\(PipeDesc \{(.*?)\}\);", render, re.S)[1]
            self.focus_fs = re.search(r'base\("blended focus depth", "vs_main", "([^"]+)"\)', descriptor)[1]
            assert '("DEPTH_CONTACT_ALPHA", FSR_BLEND_ALPHA)' in descriptor
            assert "samples: 1" in descriptor and "format: None" in descriptor
        fsr = self.shaders / "fsr"
        self.fsr = device.create_shader_module(
            code=(fsr / "common.wgsl").read_text() + (fsr / "prepare_inputs.wgsl").read_text()
        )
        mod = device.create_shader_module(code=(self.shaders / "dof.wgsl").read_text())
        entries = [
            {"binding": b, "visibility": F, "texture": {"sample_type": "depth" if b == 1 else "float"}}
            for b in range(4)
        ]
        entries.extend(
            [
                {"binding": 4, "visibility": F, "sampler": {"type": "filtering"}},
                {"binding": 5, "visibility": F, "buffer": {"type": "uniform"}},
            ]
        )
        self.lens_layout = device.create_bind_group_layout(entries=entries)
        layout = device.create_pipeline_layout(bind_group_layouts=[self.lens_layout])
        self.lens = [
            self.pipe(mod, "vs_dof", entry, formats=formats, layout=layout)
            for entry, formats in [
                ("fs_setup", [HDR]),
                ("fs_tiles", ["r16float"]),
                ("fs_dilate", ["r16float"]),
                ("fs_blur", [HDR, HDR]),
                ("fs_composite", [HDR]),
            ]
        ]
        self.sampler = device.create_sampler(mag_filter="linear", min_filter="linear")

    def tex(self, size, fmt, samples=1):
        usage = T.RENDER_ATTACHMENT
        if samples == 1:
            usage |= T.TEXTURE_BINDING | T.COPY_SRC | T.COPY_DST
        return self.d.create_texture(size=(*size, 1), sample_count=samples, format=fmt, usage=usage)

    def pipe(self, module, vs, fs, formats=(), depth=False, samples=1, constants=None, layout="auto"):
        return self.d.create_render_pipeline(
            layout=layout,
            vertex={"module": module, "entry_point": vs},
            fragment={
                "module": module,
                "entry_point": fs,
                "targets": [{"format": f} for f in formats],
                "constants": constants or {},
            },
            multisample={"count": samples},
            depth_stencil={"format": DEPTH, "depth_compare": "greater-equal", "depth_write_enabled": True}
            if depth
            else None,
        )

    def draw(self, enc, pipe, outputs=(), depth=None, bind=(), resolve=None):
        colors = [
            {
                "view": t.create_view(),
                "resolve_target": resolve.create_view() if resolve else None,
                "clear_value": (0, 0, 0, 0),
                "load_op": "clear",
                "store_op": "store",
            }
            for t in outputs
        ]
        depth_attachment = None
        if depth:
            tex, op = depth
            depth_attachment = {
                "view": tex.create_view(),
                "depth_clear_value": 0,
                "depth_load_op": op,
                "depth_store_op": "store",
            }
        group = (
            self.d.create_bind_group(
                layout=pipe.get_bind_group_layout(0), entries=[{"binding": i, "resource": v} for i, v in bind]
            )
            if bind
            else None
        )
        render = enc.begin_render_pass(color_attachments=colors, depth_stencil_attachment=depth_attachment)
        render.set_pipeline(pipe)
        if group:
            render.set_bind_group(0, group)
        render.draw(3)
        render.end()

    @staticmethod
    def mask(w, h):
        y, x = np.indices((h, w))
        # Curved hair locks, one- and two-pixel wisps, and bands of holes.
        index = (x + np.rint(2.0 * np.sin(y * 0.14)).astype(int)) % 16
        alpha = np.array([0, 0, 0, 25, 26, 0, 70, 0, 150, 150, 0, 0, 207, 0, 244, 255], dtype=np.uint8)[index]
        alpha[(y < 0.12 * h) | (y >= 0.89 * h)] = 0
        return alpha

    def boundary(self):
        mask = self.tex((2, 1), "r8unorm")
        self.d.queue.write_texture({"texture": mask}, bytes([254, 255]), {"bytes_per_row": 2}, (2, 1, 1))
        outputs = {}
        for side in ["front", "back-double", "back-single"]:
            # Full texture opacity times material alpha 0.1 equals the exact
            # pipeline cutoff. The neighbouring 254/255 pixel lies below it.
            src = self.card_source.replace("vec4u(0), vec4f(1)));", "vec4u(0), vec4f(1,1,1,0.1)));")
            if side == "back-single":
                src = src.replace("Record(vec4u(2,0,0,0)", "Record(vec4u(0,0,0,0)")
            vertex = "vs" if side == "front" else "vs_back"
            module = self.d.create_shader_module(code=src)
            pipe = self.pipe(
                module, vertex, self.focus_fs, depth=True, constants={"DEPTH_CONTACT_ALPHA": self.fsr_cutoff}
            )
            tex = self.tex((2, 1), DEPTH)
            enc = self.d.create_command_encoder()
            self.draw(enc, pipe, depth=(tex, "clear"), bind=[(0, mask.create_view())])
            self.d.queue.submit([enc.finish()])
            outputs[side] = check_depth(self.d, tex)[0].tolist()
            assert outputs[side] == ([0.0, 0.0] if side == "back-single" else [0.0, 0.5]), outputs
        return outputs

    def run(self, world, before, samples=1, fsr=True, proxy=True, motion=True, taa=False):
        d = self.d
        size = DISPLAY
        w, h = world
        alpha = self.mask(w, h)
        mask = self.tex(world, "r8unorm")
        d.queue.write_texture({"texture": mask}, alpha.tobytes(), {"bytes_per_row": w}, (*world, 1))
        enc = d.create_command_encoder()
        opaque = self.tex(world, DEPTH)
        self.draw(enc, self.pipe(self.card, "vs", "background", depth=True), depth=(opaque, "clear"))
        depth = self.tex(world, DEPTH)
        enc.copy_texture_to_texture({"texture": opaque}, {"texture": depth}, (*world, 1))
        upscaled = world != size
        fsr_written = fsr and motion
        # FSR upscaling always publishes the AA's own depth. Native FSR
        # exposes it to the lens only when its alpha-depth option is on.
        private = not before and proxy and not fsr_written
        use_ao = (before and proxy or upscaled) and not fsr_written
        use_fsr = fsr_written and (proxy or upscaled)
        if use_ao or use_fsr:
            # The old AO proxy included opaque-looking and near-surface fragments.
            # FSR's own alpha draw already included detached 10%-coverage strands.
            old_ao = use_ao
            frag = "fs_depth_occlusion" if old_ao else "fs_depth_contact"
            pipe = self.pipe(
                self.card,
                "vs",
                frag,
                depth=True,
                constants={"DEPTH_CONTACT_ALPHA": 0.5 if old_ao else self.fsr_cutoff},
            )
            bind = [(0, mask.create_view())]
            if old_ao:
                bind.append((1, opaque.create_view()))
            self.draw(enc, pipe, depth=(depth, "load"), bind=bind)
        if world != size:
            # Use FSR's real presentation-depth shader and the render-to-display constants.
            params = bytearray(208)
            np.ndarray((2,), dtype=np.int32, buffer=params, offset=64)[:] = world
            np.ndarray((2,), dtype=np.float32, buffer=params, offset=144)[:] = np.divide(world, size)
            ub = d.create_buffer_with_data(data=params, usage=B.UNIFORM)
            output = self.tex(size, DEPTH)
            pipe = self.pipe(self.fsr, "vs_fsr", "fs_display_depth", depth=True)
            self.draw(enc, pipe, depth=(output, "clear"), bind=[(0, {"buffer": ub}), (3, depth.create_view())])
            depth = output

        if private:
            lens_depth = self.tex(size, DEPTH)
            enc.copy_texture_to_texture({"texture": depth}, {"texture": lens_depth}, (*size, 1))
            pipe = self.pipe(
                self.card, "vs", self.focus_fs, depth=True, constants={"DEPTH_CONTACT_ALPHA": self.fsr_cutoff}
            )
            self.draw(enc, pipe, depth=(lens_depth, "load"), bind=[(0, mask.create_view())])
            depth = lens_depth

        sharp = self.tex(size, HDR)
        ms = self.tex(size, HDR, samples) if samples > 1 else None
        self.draw(
            enc,
            self.pipe(self.card, "vs", "color", formats=[HDR], samples=samples),
            outputs=[ms or sharp],
            resolve=sharp if ms else None,
            bind=[(0, mask.create_view())],
        )

        if taa:
            # TAA resolves before the lens. Reset first, then use that frame as
            # a static history for a second reprojection through identity.
            identity = np.eye(4, dtype=np.float32).flatten().tolist()
            motion_tex = self.tex(size, HDR)
            source = sharp
            for reset in [True, False]:
                data = np.array([*identity, *identity, float(reset), 0.1, *size, 0, 0, 0, 0], dtype=np.float32)
                uniform = d.create_buffer_with_data(data=data, usage=B.UNIFORM)
                output = self.tex(size, HDR)
                pipe = self.pipe(self.taa, "vs_taa", "fs_taa", formats=[HDR])
                self.draw(
                    enc,
                    pipe,
                    outputs=[output],
                    bind=[
                        (0, source.create_view()),
                        (1, opaque.create_view()),
                        (2, sharp.create_view()),
                        (3, self.sampler),
                        (4, {"buffer": uniform}),
                        (5, motion_tex.create_view()),
                    ],
                )
                sharp = output

        half_size = tuple((v + 1) // 2 for v in size)
        tile_size = tuple((v + 15) // 16 for v in half_size)
        half, tiles, dilate = (
            self.tex(half_size, HDR),
            self.tex(tile_size, "r16float"),
            self.tex(tile_size, "r16float"),
        )
        far, near = self.tex(half_size, HDR), self.tex(half_size, HDR)
        out = self.tex(size, HDR)
        # Column-major inverse of the infinite reverse-Z projection with near=1.
        inv = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, -1, 0]
        lens = [30, 1.5, 12, 48]
        sizes = [*size, *half_size]
        params = np.array([*inv, *lens, *sizes, *tile_size, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0], dtype=np.float32)
        uniform = d.create_buffer_with_data(data=params, usage=B.UNIFORM)
        passes = [
            ([half], sharp, tiles, tiles),
            ([tiles], half, half, half),
            ([dilate], tiles, half, half),
            ([far, near], half, dilate, tiles),
            ([out], sharp, far, near),
        ]
        for pipe, (outputs, a, b, c) in zip(self.lens, passes, strict=True):
            bind = [
                (0, a.create_view()),
                (1, depth.create_view()),
                (2, b.create_view()),
                (3, c.create_view()),
                (4, self.sampler),
                (5, {"buffer": uniform}),
            ]
            self.draw(enc, pipe, outputs=outputs, bind=bind)
        d.queue.submit([enc.finish()])

        y, x = np.indices((size[1], size[0]))
        j = np.minimum(((y + 0.5) * h / size[1]).astype(int), h - 1)
        i = np.minimum(((x + 0.5) * w / size[0]).astype(int), w - 1)
        a = alpha[j, i]
        # Avoid exact nearest-sampling ties across the two raster resolutions.
        valid = (np.abs((x + 0.5) * w / size[0] - np.round((x + 0.5) * w / size[0])) > 1.0e-4) & (
            np.abs((y + 0.5) * h / size[1] - np.round((y + 0.5) * h / size[1])) > 1.0e-4
        )
        hair = valid & (a >= 26) & (a < 224) & (x > size[0] * 0.12)
        holes = valid & (a == 0) & (x > size[0] * 0.12)
        faint = valid & (a == 25) & (x > size[0] * 0.12)
        hidden = valid & (x < size[0] * 0.06)
        z = check_depth(d, depth)
        color_err = np.mean(np.abs(check_rgba(d, out)[:, :, :3] - check_rgba(d, sharp)[:, :, :3]), axis=-1)
        result = {
            "render_size": list(world),
            "sharp_strand_fraction": float(np.mean(z[hair] == 0.5)),
            "strand_mean_rgb_change": float(np.mean(color_err[hair])),
            "holes_at_background_depth": float(np.mean(np.isclose(z[holes], 0.05))),
            "holes_mean_rgb_change": float(np.mean(color_err[holes])),
            "faint_at_background_depth": float(np.mean(np.isclose(z[faint], 0.05))),
            "occluded_hair_kept_foreground": float(np.mean(np.isclose(z[hidden], 0.6))),
        }
        expected = use_fsr or private
        assert np.all(z[hair] == (0.50 if expected else np.float32(0.05))), result
        assert result["strand_mean_rgb_change"] < 0.002 if expected else result["strand_mean_rgb_change"] > 0.04, (
            result
        )
        assert (
            result["holes_at_background_depth"]
            == result["faint_at_background_depth"]
            == result["occluded_hair_kept_foreground"]
            == 1.0
        ), result
        assert result["holes_mean_rgb_change"] > 0.06, result
        return {k: round(v, 6) if isinstance(v, float) else v for k, v in result.items()}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("root", type=Path)
    ap.add_argument("--baseline", type=Path, help="Optional previous checkout supplying the original shaders.")
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    adapter = wgpu.gpu.request_adapter_sync(power_preference="high-performance")
    device = adapter.request_device_sync(
        required_features=["texture-adapter-specific-format-features"]
        if "texture-adapter-specific-format-features" in adapter.features
        else []
    )
    fixture = Fixture(device, args.root)
    baseline = Fixture(device, args.baseline, previous=True) if args.baseline else fixture
    rows = {}
    supported = [1]
    for n in (2, 4, 8):
        try:
            for fmt in (HDR, DEPTH, "rgba8unorm"):
                fixture.tex(DISPLAY, fmt, n)
            supported.append(n)
        except wgpu.GPUValidationError as exc:
            if "sample count" not in str(exc).lower():
                raise
    for mode, ratio, requested in [
        ("None/FXAA/SMAA", 1.0, 1),
        ("TAA", 1.0, 1),
        ("MSAA 2x", 1.0, 2),
        ("MSAA 4x", 1.0, 4),
        ("MSAA 8x", 1.0, 8),
        ("FSR Native", 1.0, 1),
        ("FSR Quality", 1.5, 1),
        ("FSR Balanced", 1.7, 1),
        ("FSR Performance", 2.0, 1),
        ("FSR Ultra Performance", 3.0, 1),
    ]:
        # The renderer uses the largest compatible count no greater than
        # the requested one. FSR truncates each downscaled axis.
        samples = max(n for n in supported if n <= requested)
        world = tuple(int(v / ratio) for v in DISPLAY)
        rows[mode] = {"requested_samples": requested, "actual_samples": samples}
        for tag, before in [("before", True), ("fixed", False)]:
            runner = baseline if before else fixture
            rows[mode][tag] = runner.run(world, before, samples, fsr=mode.startswith("FSR"), taa=mode == "TAA")
    for name, ratio, is_fsr, motion, proxy in [
        ("non-FSR proxy off", 1.0, False, False, False),
        ("FSR Native proxy off", 1.0, True, True, False),
        ("FSR Quality proxy off", 1.5, True, True, False),
        ("FSR Native motion off", 1.0, True, False, True),
        ("FSR Quality motion off", 1.5, True, False, True),
    ]:
        rows[name] = fixture.run(
            tuple(int(v / ratio) for v in DISPLAY), False, fsr=is_fsr, motion=motion, proxy=proxy
        )
    result = {
        "adapter": dict(adapter.info),
        "display": DISPLAY,
        "joint_sample_counts": supported,
        "cutoff_and_sidedness": fixture.boundary(),
        "cases": rows,
    }
    print(json.dumps(result, indent=2))
    if args.json:
        args.json.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
