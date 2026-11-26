"""Replay the glasses-to-nose gap on a Vulkan adapter with a specified Aurora checkout."""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from measure import measure, verify

HERE = Path(__file__).resolve().parent
TIERS = {
    "low": (1024, 2, 48, 120, .65),
    "high": (4096, 3, 144, 120, .53),
}

def function(source, name):
    marker = re.search(r"^(?:pub\(crate\) )?fn " + name + r"\(", source, re.M)
    if not marker:
        raise ValueError(f"Missing production function {name}")
    end = source.index("\n}", marker.start()) + 2
    return source[marker.start():end]

def constant(source, name):
    match = re.search(r"^const " + name + r":[^;]+;", source, re.M)
    if not match:
        raise ValueError(f"Missing production constant {name}")
    return match[0]

def prepare(source, build):
    shader_dir = source / "crates/aurora-render/src/shaders"
    common = (shader_dir / "common.wgsl").read_text(encoding="utf-8")
    renderer = (shader_dir.parent / "renderer.rs").read_text(encoding="utf-8")
    tracer = (shader_dir.parent / "contact_shadows.rs").read_text(encoding="utf-8")
    for name in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml"):
        shutil.copyfile(HERE / name, build / name)
    src = build / "src"
    shutil.copytree(HERE / "src", src)
    (src / "shaders").mkdir()
    for name in ("contact_shadows.wgsl", "contact_shadows_filter.wgsl"):
        shutil.copyfile(shader_dir / name, src / "shaders" / name)
    (src / "production_bend.rs").write_text("\n".join(
        function(tracer, name) for name in ("build_dispatch_list", "occluder_units", "shader_source")
    ), encoding="utf-8")
    fit = [constant(renderer, name) for name in ("PORTRAIT_SIZE", "PORTRAIT_RADIUS")]
    fit.extend(function(renderer, name) for name in ("graded_detail_bounds", "shadow_anchor", "portrait_projection"))
    (src / "production_fit.rs").write_text("\n".join(fit), encoding="utf-8")
    shader = [(src / "bindings.wgsl").read_text(encoding="utf-8")]
    shader.extend(constant(common, name) for name in (
        "RC_FLAG_NEAR_SHADOW_DETAIL", "SHADOW_PENUMBRA", "SHADOW_FILTER_GAPS", "SHADOW_BIAS_LEGACY"
    ))
    shader.extend([function(common, "in_reflection_pass"),
                   common[common.index("fn shadow_taps("):common.index("// The motion of a pixel")],
                   function(common, "combine_sun_visibility"),
                   (src / "probe.wgsl").read_text(encoding="utf-8")])
    (src / "lookup.wgsl").write_text("\n".join(shader), encoding="utf-8")

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Aurora viewer source checkout")
    parser.add_argument("--tier", choices=TIERS, default="low")
    parser.add_argument("--measure", action="store_true", help="Report measurements without applying regression budgets")
    parser.add_argument("--json", type=Path, help="Also write the compact measurements as JSON")
    parser.add_argument("--capture", type=Path, help="Keep per-probe reference and visibility data as JSON")
    parser.add_argument("--depth-grid", type=Path, help="Run Bend and its filter on 384×288 reverse-Z .raw inputs; writes .bend beside each.")
    opts = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="aurora-bridge-") as tmp:
        build = Path(tmp)
        prepare(opts.source.resolve(), build)
        size, detail, distance, reach, angle = TIERS[opts.tier]
        output = opts.capture.resolve() if opts.capture else build / "capture.json"
        env = dict(os.environ, MAP_SIZE=str(size), DETAIL=str(detail), RANGES=str(distance),
                   BEND_REACH=str(reach), SUN_DIAMETER_DEG=str(angle), RESULT=str(output))
        env.pop("ULTRA_OVERLAP", None)
        # Repeated checkouts reuse dependency builds. Cargo tracks the generated source in tmp.
        env.setdefault("CARGO_TARGET_DIR", str(HERE / "target"))
        cmd = ["cargo", "run", "--locked", "--quiet"]
        if opts.depth_grid:
            cmd.extend(["--", str(opts.depth_grid.resolve())])
        subprocess.run(cmd, cwd=build, env=env, check=True)
        if opts.depth_grid:
            return
        capture = json.loads(output.read_text(encoding="utf-8"))
    report = measure(capture)
    report["tier"] = opts.tier
    rendered = json.dumps(report, indent=2) + "\n"
    if opts.json:
        opts.json.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    if not opts.measure:
        verify(report)

if __name__ == "__main__":
    main()
