"""Pure rules of the shape transfer: no Maya import, unit-tested alone.

Aurora's "Add a skeleton with my shape" sends the worn shape as the 159 SL
joints at its proportions and a neutral reference body
(crates/aurora-viewer/src/blender_live/shape.rs). The checks follow the Blender
plug-in's (blender/aurora_live/shape_transfer.py): only bounded numbers and
names are accepted, never a path, script or existing object.
"""

import math
import uuid
from pathlib import Path
import xml.etree.ElementTree as ET

from . import convert

PARTS = {"head", "upper", "lower", "eye_left", "eye_right", "eyelashes"}
MAX_VERTICES = 50000
MAX_TRIANGLES = 100000


def skeleton(path=None):
    """(name, parent name) of the SL joints in pre-order, collision volumes
    included, from the bundled avatar_skeleton.xml."""
    if path is None:
        path = Path(__file__).with_name("avatar_skeleton.xml")
        if not path.is_file():
            # Source checkout; the package carries the same file beside this one.
            path = Path(__file__).resolve().parents[2] / "crates/aurora-viewer/assets/character/avatar_skeleton.xml"
    result = []

    def walk(node, parent=None):
        if node.tag not in ("bone", "collision_volume"):
            return
        result.append((node.attrib["name"], parent))
        for child in node:
            walk(child, node.attrib["name"])

    for node in ET.parse(str(path)).getroot():
        walk(node)
    if len(result) != 159 or len({name for name, _parent in result}) != 159:
        raise ValueError("The bundled SL skeleton must contain 133 bones and 26 collision volumes")
    return result


def _numbers(value, length):
    if not isinstance(value, (list, tuple)) or len(value) != length:
        raise ValueError("Invalid shape vector")
    if any(type(number) not in (int, float) or not math.isfinite(number) or abs(number) > 10000 for number in value):
        raise ValueError("Invalid shape number")
    return [float(number) for number in value]


def validate(payload, joints):
    """Check a shape transfer against the SL skeleton `joints` (from
    skeleton()); raise ValueError on anything out of bounds."""
    if not isinstance(payload, dict):
        raise ValueError("Invalid shape transfer")
    if payload.get("action", "create") not in ("create", "apply"):
        raise ValueError("Unsupported shape action")
    uuid.UUID(str(payload.get("id")))
    name = payload.get("name")
    if not isinstance(name, str) or len(name) > 128:
        raise ValueError("Invalid shape name")
    names = [joint for joint, _parent in joints]
    if payload.get("names") != names or not isinstance(payload.get("joints"), list) or len(payload["joints"]) != len(names):
        raise ValueError("Incompatible SL shape skeleton")
    for expected, joint in zip(names, payload["joints"]):
        if not isinstance(joint, dict) or joint.get("name") != expected:
            raise ValueError("Incompatible SL shape joint")
        head = _numbers(joint.get("head"), 3)
        _numbers(joint.get("tail"), 3)
        x, y, z, w = _numbers(joint.get("rotation"), 4)
        if abs(x * x + y * y + z * z + w * w - 1) > 0.001:
            raise ValueError("Invalid shape rotation")
        inverse = convert.from_column_major(_numbers(joint.get("inverse_bind"), 16))
        if abs(convert.determinant3(inverse)) < 1e-8 or math.dist(convert.point(inverse, head), (0, 0, 0)) > 0.0001:
            raise ValueError("Invalid shape bind matrix")
        if payload.get("action") == "apply":
            reference = convert.from_column_major(_numbers(joint.get("reference_bind"), 16))
            if abs(convert.determinant3(reference)) < 1e-8:
                raise ValueError("Invalid reference shape bind")
    params = payload.get("params")
    if not isinstance(params, dict) or len(params) > 512:
        raise ValueError("Invalid shape parameters")
    for key, value in params.items():
        if not str(key).isdigit():
            raise ValueError("Invalid shape parameter id")
        _numbers([value], 1)
    parts = payload.get("parts")
    if not isinstance(parts, list) or not 1 <= len(parts) <= 6:
        raise ValueError("Invalid shape body parts")
    vertices = 0
    for part in parts:
        if not isinstance(part, dict) or part.get("name") not in PARTS:
            raise ValueError("Invalid shape body part")
        positions, uvs, weights = part.get("positions"), part.get("uvs"), part.get("weights")
        count = len(positions) if isinstance(positions, list) else 0
        vertices += count
        if (not count or vertices > MAX_VERTICES or not isinstance(uvs, list) or len(uvs) != count
                or not isinstance(weights, list) or len(weights) != count):
            raise ValueError("Invalid shape body vertex count")
        for position, uv, groups in zip(positions, uvs, weights):
            _numbers(position, 3)
            _numbers(uv, 2)
            if not isinstance(groups, list) or not 1 <= len(groups) <= 4:
                raise ValueError("Invalid shape weights")
            total = 0.0
            for group in groups:
                if not isinstance(group, (list, tuple)) or len(group) != 2:
                    raise ValueError("Invalid shape influence")
                index, weight = group
                if type(index) is not int or not 0 <= index < len(names) or not 0 < _numbers([weight], 1)[0] <= 1:
                    raise ValueError("Invalid shape influence")
                total += weight
            if abs(total - 1) > 0.001:
                raise ValueError("Invalid shape weight sum")
        triangles = part.get("triangles")
        if not isinstance(triangles, list) or not triangles or len(triangles) > MAX_TRIANGLES:
            raise ValueError("Invalid shape triangle count")
        for triangle in triangles:
            if (not isinstance(triangle, (list, tuple)) or len(triangle) != 3
                    or any(type(index) is not int or not 0 <= index < count for index in triangle)):
                raise ValueError("Invalid shape triangle")
    return payload


def placement(up_axis, unit):
    """SL metres to Maya units for a new skeleton: the inverse of the frame a
    capture finds for it. A Y-up scene gets the rig facing Maya's +Z, the way
    Second Life rigs in Maya usually stand (left on +X); a Z-up scene keeps
    SL's axes. `unit` is metres per Maya unit."""
    frame = convert.base_frame(up_axis, unit)
    if str(up_axis).lower() == "y":
        frame = convert.multiply(convert.rotation_z(math.pi / 2), frame)
    return convert.inverse(frame)


def node_name(name):
    """A Maya node name from the shape's name: letters, digits and _ only."""
    cleaned = "".join(character if character.isascii() and (character.isalnum() or character == "_") else "_"
                      for character in name).strip("_") or "Shape"
    return ("Shape_" + cleaned)[:60] if not cleaned[0].isalpha() else cleaned[:60]
