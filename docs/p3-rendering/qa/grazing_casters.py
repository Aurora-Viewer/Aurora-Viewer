"""Ground truth for grazing shadows from extruded cross-sections.

Run from the root of a checkout containing this change with Python, NumPy, SciPy, Cargo and Vulkan:
python docs/p3-rendering/qa/grazing_casters.py .

A 1.2 mm diameter wire sits above the receiver; a welded ridge and a step
touch it. The source is written in tangent-plane coordinates so camera rays
and light rays can intersect the same cross-section independently. "core"
is mean filtered Bend occlusion where a light ray hits the caster; "lit"
is the mean on visible receiver pixels where that ray escapes. Eroding
both groups excludes mixed coverage at the shadow boundary. With --baseline
the same reference checkout runs on the same adapter. Losing more than four
percentage points of mean occlusion in any true shadow core fails the check.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import numpy as np

H, W = 288, 384
F = H / (2 * np.tan(np.deg2rad(30)))
Y, X = np.indices((H, W))
RAYS = np.stack(((X + 0.5 - W / 2) / F, (H / 2 - Y - 0.5) / F, -np.ones_like(X)), axis=-1)
CASES = [
    (kind, tilt, dist, nl)
    for kind in ["wire", "weld", "lip"]
    for tilt, dist, nl in [(25, 0.16, 0.05), (55, 0.23, 0.018), (73, 0.12, 0.05)]
]


def scene(kind, tilt, dist, nl):
    a = np.deg2rad(tilt)
    n = np.array([0.75 * np.sin(a), -0.6614378 * np.sin(a), np.cos(a)])
    n /= np.linalg.norm(n)
    p = np.array([0.005, -0.007, -dist])
    t = np.cross(n, [0, 1, 0])
    t /= np.linalg.norm(t)
    b = np.cross(n, t)
    angle = np.deg2rad(62)
    u = np.cos(angle) * t + np.sin(angle) * b
    v = np.cross(n, u)
    light = np.sqrt(1 - nl * nl) * u + nl * n
    ro = (-p) @ np.array([u, v, n]).T
    rd = RAYS @ np.array([u, v, n]).T
    with np.errstate(divide="ignore", invalid="ignore"):
        z = -ro[2] / rd[:, :, 2]
    valid = (z > 0.011) & (z < 0.45) & (rd[:, :, 2] < 0)
    plane = np.where(valid, z, 100)
    qu = ro[0] + plane * rd[:, :, 0]
    qv = ro[1] + plane * rd[:, :, 1]
    if kind == "wire":
        theta = np.linspace(0, 2 * np.pi, 37)
        verts = np.stack((0.0006 * np.cos(theta), 0.00125 + 0.0006 * np.sin(theta)), axis=-1)
    elif kind == "weld":
        verts = np.array([(-0.004, 0), (-0.0003, 0.0018), (0.001, 0.0018), (0.0045, 0)])
    else:
        verts = np.array([(-0.003, 0), (-0.003, 0.0012), (0.002, 0.0012), (0.002, 0)])

    def hit(o, vdir, o_height, dz):
        nearest = np.full_like(plane, np.inf)
        for (ux, hx), (uy, hy) in zip(verts, verts[1:]):
            width = uy - ux
            height = hy - hx
            den = vdir * height - dz * width
            den = np.where(abs(den) < 1e-12, np.nan, den)
            tt = ((ux - o) * height - (hx - o_height) * width) / den
            fraction = ((ux - o) * dz - (hx - o_height) * vdir) / den
            good = (tt > 1.0e-7) & (fraction >= 0) & (fraction <= 1)
            nearest = np.fmin(nearest, np.where(good, tt, np.inf))
        return nearest

    slab = hit(ro[0], rd[:, :, 0], ro[2], rd[:, :, 2])
    slab = np.where(abs(ro[1] + slab * rd[:, :, 1]) < 0.06, slab, np.inf)
    view = np.minimum(plane, slab)
    depth = np.where(view < 0.45, 0.01 / view, 0)
    # Whether the sun ray from that exact visible floor pixel hits any face of the extrusion.
    truth = np.isfinite(hit(qu, np.sqrt(1 - nl * nl), 0, nl)) & (abs(qv) < 0.06)
    # Exclude depth-border neighbours and the extrusion itself for classification.
    safe = valid & (plane <= slab) & (abs(qv) < 0.03) & (X > 20) & (X < W - 20) & (Y > 20) & (Y < H - 20)
    from scipy.ndimage import binary_erosion

    core = binary_erosion(safe & truth, iterations=2)
    lit = binary_erosion(safe & ~truth, iterations=3)
    return light, depth, (lit, core)


def run(source, runner, folder):
    regions = {}
    for kind, tilt, d, nl in CASES:
        label = f"{kind}-{tilt}"
        light, z, pixels = scene(kind, tilt, d, nl)
        with (folder / (label + ".raw")).open("wb") as fp:
            light.astype("<f4").tofile(fp)
            z.astype("<f4").tofile(fp)
        regions[label] = pixels
    subprocess.run([sys.executable, str(runner), str(source), "--depth-grid", str(folder)], check=True)
    out = {}
    for label, (lit, core) in regions.items():
        occ = np.fromfile(folder / (label + ".bend"), np.uint8).reshape(H, W) / 255
        out[label] = {
            "core_pixels": int(core.sum()),
            "lit_pixels": int(lit.sum()),
            "core": round(float(occ[core].mean()), 4),
            "lit": round(float(occ[lit].mean()), 4),
            "core_over10": round(float((occ[core] > 0.1).mean()), 4),
            "lit_over10": round(float((occ[lit] > 0.1).mean()), 4),
        }
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Aurora checkout")
    parser.add_argument("--runner", type=Path, default=Path(__file__).resolve().parent / "bridge-pullback" / "run.py",
                        help="Use a different copy of the bridge-pullback runner")
    parser.add_argument("--baseline", type=Path, help="Also measure the previous Aurora source")
    parser.add_argument("--json", type=Path, help="Write the results to this file")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="aurora-grazing-casters-") as scratch:
        folder = Path(scratch)
        results = {"candidate": run(args.source, args.runner, folder)}
        if args.baseline:
            results["baseline"] = run(args.baseline, args.runner, folder)
    for label, row in results["candidate"].items():
        # Each physical occluder must retain measurable occlusion in its
        # independently ray-tested core, clearly darker than the lit area.
        assert row["core_pixels"] > 500 and row["lit_pixels"] > 2000, label
        assert row["core"] >= 0.025 and row["core"] > 1.8 * row["lit"], (label, row)
        if "baseline" in results:
            original = results["baseline"][label]["core"]
            assert row["core"] >= original - 0.04, (label, original, row["core"])
    text = json.dumps(results, indent=2)
    print(text)
    if args.json:
        args.json.write_text(text + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
