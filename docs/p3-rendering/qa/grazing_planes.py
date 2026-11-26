"""Measure false contact shadows on single planes with known light incidence.

From the root of a checkout containing this change:
python docs/p3-rendering/qa/grazing_planes.py .

Requires numpy, scipy, Cargo and a Vulkan adapter. The 72 views include 54
light-facing planes with no caster, and 18 facing away from the light. A working
tracer leaves the first group lit and still detects the solid in the second.
Add --curved to exercise 24 views of convex meshes with different facet widths.
Their visible, light-facing facets have no blocker in the light direction.
"""

import argparse
from itertools import product
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
from scipy.ndimage import binary_erosion


WIDTH, HEIGHT = 384, 288
FOCAL = HEIGHT / (2 * np.tan(np.deg2rad(30)))
Y, X = np.indices((HEIGHT, WIDTH))
RAYS = np.stack(((X + 0.5 - WIDTH / 2) / FOCAL, (HEIGHT / 2 - Y - 0.5) / FOCAL, -np.ones_like(X)), axis=-1)
INCIDENCES = (-0.04, 0.001, 0.018, 0.05)


def generate(folder):
    cases = {}
    for tilt, distance in product((25, 55, 73), (0.08, 0.23)):
        a = np.deg2rad(tilt)
        normal = np.array([0.75 * np.sin(a), -0.6614378 * np.sin(a), np.cos(a)])
        normal /= np.linalg.norm(normal)
        point = np.array([0.005, -0.007, -distance])
        # For a camera ray t*r, intersect the plane N.(t*r - point) = 0.
        dot = RAYS @ normal
        camera_z = (normal @ point) / np.minimum(dot, -1e-5)
        visible = (dot < 0) & (camera_z > 0.01) & (camera_z < 0.35)
        depth = np.where(visible, 0.01 / np.maximum(camera_z, 0.01), 0)
        tangent = np.cross(normal, [0, 1, 0])
        tangent /= np.linalg.norm(tangent)
        bitangent = np.cross(normal, tangent)
        # Exclude the clipping boundary and a five-pixel band around any sky.
        interior = visible & (X >= 12) & (X < WIDTH-12) & (Y >= 12) & (Y < HEIGHT-12)
        interior = binary_erosion(interior, iterations=5)
        for azimuth, incidence in product((10, 52, 89), INCIDENCES):
            angle = np.deg2rad(azimuth)
            along = np.cos(angle) * tangent + np.sin(angle) * bitangent
            light = np.sqrt(1 - incidence ** 2) * along + incidence * normal
            name = f"tilt{tilt}-cm{distance*100:.0f}-az{azimuth}-nl{incidence:g}"
            with (folder / f"{name}.raw").open("wb") as stream:
                light.astype("<f4").tofile(stream)
                depth.astype("<f4").tofile(stream)
            cases[name] = (incidence, interior)
    return cases


def generate_curved(folder):
    # Each planar strip of the convex mesh joins adjacent vertices of
    # h(u) = -curvature*u*u. The surface is extruded along camera Y.
    cases = {}
    center = np.array([0.0, 0.0, -0.27])
    for tilt, curvature, width in product((-50, -28, 18, 46), (0.65, 2.2), (0.04, 0.01, 0.0025)):
        angle = np.deg2rad(tilt)
        normal = np.array([np.sin(angle), 0, np.cos(angle)])
        axis = np.array([np.cos(angle), 0, -np.sin(angle)])
        tangent = 0.76 * axis + np.array([0, 0.65, 0])
        light = tangent / np.linalg.norm(tangent) + 0.03 * normal
        light /= np.linalg.norm(light)

        # Intersect each camera column with every strip, then select the
        # nearest positive intersection within that strip's endpoints.
        left = np.arange(-0.6, 0.6, width)[:, None]
        slope = -curvature * (2 * left + width)
        intercept = curvature * left * (left + width)
        du = RAYS[0] @ axis
        dh = RAYS[0] @ normal
        with np.errstate(divide="ignore", invalid="ignore"):
            distance = (intercept + center @ normal - slope * (center @ axis)) / (dh - slope * du)
            u = distance * du - center @ axis
        valid = (distance > 0.025) & (distance < 0.8) & (u >= left) & (u <= left + width)
        distance = np.where(valid, distance, np.inf)
        nearest = distance.argmin(axis=0)
        depth = 0.01 / distance[nearest, np.arange(WIDTH)]
        facet_normals = normal - slope * axis
        incidence = (facet_normals @ light) / np.linalg.norm(facet_normals, axis=1)
        lit = (depth > 0) & (incidence[nearest] > 0.002) & (incidence[nearest] < 0.5)
        interior = lit & (X >= 40) & (X < WIDTH - 40) & (Y >= 40) & (Y < HEIGHT - 40)
        name = f"curve{curvature:g}-width{width:g}-tilt{tilt}"
        with (folder / f"{name}.raw").open("wb") as stream:
            light.astype("<f4").tofile(stream)
            np.broadcast_to(depth, (HEIGHT, WIDTH)).astype("<f4").tofile(stream)
        cases[name] = (width, interior)
    return cases


def measure(folder, cases):
    measured = {}
    for name, (incidence, mask) in cases.items():
        frame = np.fromfile(folder / f"{name}.bend", np.uint8).reshape(HEIGHT, WIDTH)
        occlusion = frame[mask].mean() / 255
        measured.setdefault(incidence, {})[name] = float(occlusion)
    result = {}
    for incidence, poses in measured.items():
        worst = min(poses, key=poses.get) if incidence < 0 else max(poses, key=poses.get)
        result[incidence] = dict(poses=len(poses), mean=float(np.mean(list(poses.values()))),
                                 worst=poses[worst], worst_pose=worst)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path, help="Aurora checkout")
    parser.add_argument("--runner", type=Path, default=Path(__file__).resolve().parent / "bridge-pullback" / "run.py",
                        help="Use a different copy of the bridge-pullback runner")
    parser.add_argument("--curved", action="store_true", help="Group convex receivers by facet width in metres")
    parser.add_argument("--measure", action="store_true", help="Record results without checking regression budgets")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="aurora-grazing-planes-") as tmp:
        folder = Path(tmp)
        cases = generate_curved(folder) if args.curved else generate(folder)
        subprocess.run([sys.executable, str(args.runner), str(args.source), "--depth-grid", str(folder)], check=True)
        result = measure(folder, cases)
    report = json.dumps(result, indent=2)
    print(report)
    if args.json:
        args.json.write_text(report + "\n", encoding="utf-8")
    if not args.measure:
        for incidence, row in result.items():
            if incidence < 0:
                assert row["worst"] > 0.85, row
            else:
                assert row["worst"] < 0.05 and row["mean"] < 0.01, row


if __name__ == "__main__":
    main()
