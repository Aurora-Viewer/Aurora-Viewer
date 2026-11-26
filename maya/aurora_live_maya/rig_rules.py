"""Pure rules of the Maya rig tools: no Maya import, unit-tested alone.

Ports of the Blender plug-in's starter and controls (blender/aurora_live/
starter.py, rigging.py): the default SL skeleton from avatar_skeleton.xml
(joint placement as LLAvatarAppearance::setupBone and LLXformMatrix, originally
LGPL 2.1), the display groups of its joints, the rigged test T-shirt and the
pole placement of hand/foot IK. Positions are in SL metres (Z up, facing +X);
the Maya modules place them with shape_rules.placement.
"""

import math
from pathlib import Path
import xml.etree.ElementTree as ET

# ------------------------------------------------------------ quaternions


def _quaternion_euler_xyz(x, y, z):
    """Blender's Euler((x, y, z), "XYZ").to_quaternion(), as (w, x, y, z)."""
    cx, sx = math.cos(x / 2), math.sin(x / 2)
    cy, sy = math.cos(y / 2), math.sin(y / 2)
    cz, sz = math.cos(z / 2), math.sin(z / 2)
    return (cx * cy * cz + sx * sy * sz, sx * cy * cz - cx * sy * sz,
            cx * sy * cz + sx * cy * sz, cx * cy * sz - sx * sy * cz)


def _multiply(a, b):
    aw, ax, ay, az = a
    bw, bx, by, bz = b
    return (aw * bw - ax * bx - ay * by - az * bz, aw * bx + ax * bw + ay * bz - az * by,
            aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw)


def _rotate(q, v):
    w, x, y, z = q
    vx, vy, vz = v
    # v + 2w(u x v) + 2u x (u x v), u = (x, y, z)
    cx, cy, cz = y * vz - z * vy, z * vx - x * vz, x * vy - y * vx
    return (vx + 2 * (w * cx + y * cz - z * cy), vy + 2 * (w * cy + z * cx - x * cz),
            vz + 2 * (w * cz + x * cy - y * cx))


# ---------------------------------------------------------------- skeleton

def _definition(path=None):
    if path is None:
        path = Path(__file__).with_name("avatar_skeleton.xml")
        if not path.is_file():
            path = Path(__file__).resolve().parents[2] / "crates/aurora-viewer/assets/character/avatar_skeleton.xml"
    return ET.parse(str(path)).getroot()


def default_joints(path=None):
    """The SL rest skeleton in pre-order, collision volumes included: name,
    parent, head and tail (SL metres), group, support and collision flag.
    Same walk as the Blender starter's joints()."""
    result = []

    def walk(node, parent=None, position=(0.0, 0.0, 0.0), rotation=(1.0, 0.0, 0.0, 0.0), scale=(1.0, 1.0, 1.0)):
        if node.tag not in ("bone", "collision_volume"):
            return
        local = [float(v) for v in node.attrib["pos"].split()]
        angles = [math.radians(float(v)) for v in node.attrib["rot"].split()]
        own_scale = tuple(float(v) for v in node.attrib["scale"].split())
        offset = _rotate(rotation, tuple(local[i] * scale[i] for i in range(3)))
        position = tuple(position[i] + offset[i] for i in range(3))
        rotation = _multiply(rotation, _quaternion_euler_xyz(*angles))
        end = [float(v) for v in node.get("end", "0 0 0.03").split()]
        tail = tuple(position[i] + _rotate(rotation, end)[i] for i in range(3))
        result.append({"name": node.attrib["name"], "parent": parent, "head": position, "tail": tail,
                       "group": node.get("group", "Extra"), "support": node.get("support", "base"),
                       "collision": node.tag == "collision_volume"})
        for child in node:
            walk(child, node.attrib["name"], position, rotation, own_scale)

    for node in _definition(path):
        walk(node)
    if len(result) != 159:
        raise ValueError("The bundled SL skeleton must contain 133 bones and 26 collision volumes")
    return result


# The four Bento joints folded over the classic spine (mPelvis -> mSpine1 ->
# mSpine2 -> mTorso -> mSpine3 -> mSpine4 -> mChest): they overlap it at rest.
FOLDED_SPINE = ("mSpine1", "mSpine2", "mSpine3", "mSpine4")


def layer(joint):
    """The display layer of a joint, the Blender starter's bone collections."""
    if joint["collision"]:
        return "Fitted_Mesh"
    if joint["support"] == "base":
        return "Body"
    group = joint["group"]
    if group in ("Face", "Eyes", "Ears", "Mouth", "Lips"):
        return "Bento_Face"
    if "Hand" in group:
        return "Bento_Hands"
    return "Bento_" + "".join(c if c.isalnum() else "_" for c in group)


VISIBLE_LAYERS = ("Body", "Bento_Hands")


def weight_joints(joints, mode):
    """Joints automatic weights may use: "body" the classic SL body joints;
    "all" every deforming joint but collision volumes and the folded spine
    (Blender plug-in, Automatic weight painting)."""
    if mode == "body":
        return [j["name"] for j in joints if j["support"] == "base" and not j["collision"]]
    return [j["name"] for j in joints if not j["collision"] and j["name"] not in FOLDED_SPINE]


# --------------------------------------------------------------- T-shirt

def _clamp(value):
    return max(0.0, min(1.0, value))


def _shirt_weights(y, z, heads):
    pelvis, torso, chest = (heads[name][2] for name in ("mPelvis", "mTorso", "mChest"))
    if z < torso:
        t = _clamp((z - pelvis) / (torso - pelvis))
        weights = {"mPelvis": 1 - t, "mTorso": t}
    else:
        t = _clamp((z - torso) / (chest - torso))
        weights = {"mTorso": 1 - t, "mChest": t}
    side = "Left" if y >= 0 else "Right"
    shoulder = heads["mShoulder" + side]
    arm = _clamp((abs(y) - 0.10) / 0.18) * _clamp((z - (shoulder[2] - 0.20)) / 0.13)
    elbow = 0.55 * _clamp((abs(y) - abs(shoulder[1]) - 0.13) / 0.16)
    weights = {name: weight * (1 - arm) for name, weight in weights.items()}
    weights.update({"mCollar" + side: arm * 0.15 * (1 - elbow), "mShoulder" + side: arm * 0.85 * (1 - elbow),
                    "mElbow" + side: arm * elbow})
    values = sorted(((name, weight) for name, weight in weights.items() if weight > 0.000001),
                    key=lambda pair: (-pair[1], pair[0]))[:4]
    total = sum(weight for _, weight in values)
    return [(name, weight / total) for name, weight in values]


def tshirt(heads):
    """The Blender starter's test T-shirt: two quad panels sewn at the
    shoulders and sides, neck, hem and cuffs open. Returns SL-metre vertices,
    quads (vertex indices), a UV per quad corner and each vertex's weights."""
    bottom = heads["mPelvis"][2] - 0.025
    shoulder = heads["mShoulderLeft"][2]
    underarm, top = shoulder - 0.13, shoulder + 0.065
    vertices, faces, uvs, weights, panels = [], [], [], [], []
    for front in (True, False):
        panel = {}
        for row in range(11):
            columns = range(-8, 9) if row < 6 else range(-16, 17)
            for col in columns:
                y = col * 0.03
                if row < 6:
                    z = bottom + (underarm - bottom) * row / 6
                    y *= 0.91 + 0.09 * math.sin(math.pi * row / 6)
                else:
                    neckline = max(0, 1 - (abs(y) / 0.09) ** 2)
                    edge = top - (0.085 if front else 0.025) * neckline
                    z = underarm + (edge - underarm) * (row - 6) / 4
                sleeve = _clamp((abs(y) - 0.18) / 0.12)
                depth = (0.18 if front else 0.16) * (1 - sleeve) + 0.085 * sleeve
                depth *= 1 - 0.14 * min(1, (abs(y) / 0.24) ** 2) * (1 - sleeve)
                panel[row, col] = len(vertices)
                vertices.append((depth if front else -depth, y, z))
                weights.append(_shirt_weights(y, z, heads))
        for (row, col), _index in panel.items():
            keys = ((row, col), (row, col + 1), (row + 1, col + 1), (row + 1, col))
            if all(key in panel for key in keys):
                # Front faces wind toward +X, back faces toward -X.
                ordered = keys if front else tuple(reversed(keys))
                faces.append([panel[key] for key in ordered])
                uvs.append([((key[1] + 16) / 64 + (0 if front else 0.5), key[0] / 10) for key in ordered])
        panels.append(panel)
    front, back = panels
    seams = []
    for side in (-1, 1):
        seams.extend([((row, side * 8), (row + 1, side * 8)) for row in range(6)])
        seams.extend([((6, col), (6, col + 1)) for col in (range(8, 16) if side == 1 else range(-16, -8))])
        seams.extend([((10, col), (10, col + 1)) for col in (range(3, 16) if side == 1 else range(-16, -3))])
    sleeve_axis = (underarm + top) / 2
    for a, b in seams:
        face, corners = [front[a], front[b], back[b], back[a]], [(0, 0), (1, 0), (1, 1), (0, 1)]
        # Point outward from the tube the seam closes: the torso's vertical
        # axis, or a sleeve's axis along Y at mid-sleeve height.
        points = [vertices[index] for index in face]
        centre = tuple(sum(p[axis] for p in points) / 4 for axis in range(3))
        sleeve = abs(centre[1]) > 0.24
        reference = (0.0, centre[1] if sleeve else 0.0, sleeve_axis if sleeve else centre[2])
        normal = _cross(_sub(points[1], points[0]), _sub(points[2], points[1]))
        if _dot(normal, _sub(centre, reference)) < 0:
            face, corners = list(reversed(face)), list(reversed(corners))
        faces.append(face)
        uvs.append(corners)
    return vertices, faces, uvs, weights


# --------------------------------------------------------------------- IK

LIMBS = {
    "HandLeft": ("mShoulderLeft", "mElbowLeft", "mWristLeft"),
    "HandRight": ("mShoulderRight", "mElbowRight", "mWristRight"),
    "FootLeft": ("mHipLeft", "mKneeLeft", "mAnkleLeft"),
    "FootRight": ("mHipRight", "mKneeRight", "mAnkleRight"),
}


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _length(a):
    return math.sqrt(_dot(a, a))


def _normalized(a):
    length = _length(a)
    if length < 1e-12:
        raise ValueError("Zero-length direction")
    return tuple(x / length for x in a)


def pole_position(heads, limb, up=(0.0, 0.0, 1.0)):
    """Where a limb's pole goes: the Blender plug-in's rule. Elbow poles sit
    behind the arm at elbow height, two upper-arm lengths away; knee poles
    ahead of the knee, 0.6 thigh lengths away. Facing comes from the
    shoulders, not from joint orientation. A clearly bent limb keeps its bend
    plane. `heads` are world positions in any one frame, `up` the rig's up."""
    root, middle, end = (heads[name] for name in LIMBS[limb])
    left, right = heads["mShoulderLeft"], heads["mShoulderRight"]
    direction = _cross(_normalized(_sub(left, right)), _normalized(up))
    hand = limb.startswith("Hand")
    if hand:
        direction = tuple(-x for x in direction)
    distance = _length(_sub(middle, root)) * (2 if hand else 0.6)
    axis = _sub(end, root)
    axis_length = _length(axis)
    if axis_length < 1e-6:
        raise ValueError("The IK chain has zero length")
    unit = tuple(x / axis_length for x in axis)
    offset = _sub(middle, root)
    perpendicular = _sub(offset, tuple(u * _dot(offset, unit) for u in unit))
    if _length(perpendicular) > axis_length * 0.05:
        direction = perpendicular
    direction = _normalized(direction)
    return tuple(m + d * distance for m, d in zip(middle, direction))
