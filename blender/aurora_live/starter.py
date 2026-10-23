"""Build an editable SL rest skeleton and a small, deterministic test garment.

Joint placement follows LLAvatarAppearance::setupBone and LLXformMatrix
(originally LGPL 2.1). The packaged XML is Aurora's unchanged avatar definition;
see SKELETON-NOTICE.txt. Blender's bone roll is only an editing aid.
"""

import math
from pathlib import Path
import xml.etree.ElementTree as ET

import bpy
import bmesh
from mathutils import Euler, Matrix, Quaternion, Vector
from .compat import organize_bones


def _definition():
    path = Path(__file__).with_name("avatar_skeleton.xml")
    if not path.is_file():
        # Source checkout; the packagers embed the same file beside this module.
        path = Path(__file__).resolve().parents[2] / "crates/aurora-viewer/assets/character/avatar_skeleton.xml"
    return ET.parse(path).getroot()


def joints():
    """Return pre-order SL joints, including collision volumes, in metres."""
    result = []

    def walk(node, parent=None, position=Vector(), rotation=Quaternion(), scale=Vector((1, 1, 1))):
        if node.tag not in {"bone", "collision_volume"}:
            return
        local = Vector(tuple(float(v) for v in node.attrib["pos"].split()))
        angles = tuple(math.radians(float(v)) for v in node.attrib["rot"].split())
        own_scale = Vector(tuple(float(v) for v in node.attrib["scale"].split()))
        position = position + rotation @ Vector(tuple(local[i] * scale[i] for i in range(3)))
        rotation = rotation @ Euler(angles, "XYZ").to_quaternion()
        end = Vector(tuple(float(v) for v in node.get("end", "0 0 0.03").split()))
        result.append({
            "name": node.attrib["name"], "parent": parent, "head": position,
            "tail": position + rotation @ end, "rotation": rotation,
            "scale": own_scale, "group": node.get("group", "Extra"),
            "collision": node.tag == "collision_volume", "support": node.get("support", "base"),
            "connected": node.get("connected") == "true",
        })
        for child in node:
            walk(child, node.attrib["name"], position, rotation, own_scale)

    for node in _definition():
        walk(node)
    if len(result) != 159 or len({joint["name"] for joint in result}) != 159:
        raise ValueError("The bundled SL skeleton must contain 133 bones and 26 collision volumes")
    return result


def _basis(context, forward):
    unit = context.scene.unit_settings.scale_length
    if not math.isfinite(unit) or unit <= 0:
        raise ValueError("Scene unit scale must be positive")
    turn = Matrix.Rotation(-math.pi / 2, 4, "Z") if forward == "NEGATIVE_Y" else Matrix.Identity(4)
    return turn @ Matrix.Scale(1.0 / unit, 4)


def _collection_name(joint):
    if joint["collision"]:
        return "Fitted Mesh"
    if joint["support"] == "base":
        return "Body"
    group = joint["group"]
    if group in {"Face", "Eyes", "Ears", "Mouth", "Lips"}:
        return "Bento Face"
    if "Hand" in group:
        return "Bento Hands"
    return "Bento " + group


def _rig(context, collection, definitions, basis):
    data = bpy.data.armatures.new("SL Avatar Rig")
    rig = bpy.data.objects.new("SL Avatar Rig", data)
    collection.objects.link(rig)
    rig.location = context.scene.cursor.location
    rig.show_in_front = True
    data.display_type = "OCTAHEDRAL"
    rig["aurora_starter"] = "Second Life Bento / Fitted Mesh"
    context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    try:
        for joint in definitions:
            bone = data.edit_bones.new(joint["name"])
            bone.head = basis @ joint["head"]
            bone.tail = basis @ joint["tail"]
            if (bone.tail - bone.head).length < 0.00001:
                bone.tail = bone.head + basis.to_3x3() @ Vector((0, 0, 0.03))
            bone.align_roll(basis.to_3x3() @ (joint["rotation"] @ Vector((0, 0, 1))))
            if joint["parent"]:
                bone.parent = data.edit_bones[joint["parent"]]
                # Some Bento spine joints double back; never snap XML heads.
                bone.use_connect = joint["connected"] and (bone.parent.tail - bone.head).length < 0.000001
            bone.use_deform = True
    finally:
        bpy.ops.object.mode_set(mode="OBJECT")
    assignments = []
    for joint in definitions:
        bone = data.bones[joint["name"]]
        bone["aurora_joint"] = joint["name"]
        bone["sl_collision_volume"] = joint["collision"]
        bone["sl_rest_scale"] = tuple(joint["scale"])
        assignments.append((bone, _collection_name(joint)))
    organize_bones(rig, assignments)
    return rig


def _clamp(value):
    return max(0.0, min(1.0, value))


def _weights(y, z, heads):
    pelvis, torso, chest = (heads[name].z for name in ("mPelvis", "mTorso", "mChest"))
    # Torso interpolation is independent of row count or mesh subdivision.
    if z < torso:
        t = _clamp((z - pelvis) / (torso - pelvis))
        weights = {"mPelvis": 1 - t, "mTorso": t}
    else:
        t = _clamp((z - torso) / (chest - torso))
        weights = {"mTorso": 1 - t, "mChest": t}
    side = "Left" if y >= 0 else "Right"
    shoulder = heads["mShoulder" + side]
    arm = _clamp((abs(y) - 0.10) / 0.18) * _clamp((z - (shoulder.z - 0.20)) / 0.13)
    elbow = 0.55 * _clamp((abs(y) - abs(shoulder.y) - 0.13) / 0.16)
    weights = {name: weight * (1 - arm) for name, weight in weights.items()}
    weights.update({
        "mCollar" + side: arm * 0.15 * (1 - elbow),
        "mShoulder" + side: arm * 0.85 * (1 - elbow),
        "mElbow" + side: arm * elbow,
    })
    values = sorted(((name, weight) for name, weight in weights.items() if weight > 0.000001),
                    key=lambda pair: (-pair[1], pair[0]))[:4]
    total = sum(weight for _, weight in values)
    return [(name, weight / total) for name, weight in values]


def _tshirt(collection, rig, definitions, basis):
    """Two quad panels, sewn at shoulders/sides: neck, hem and cuffs stay open."""
    heads = {joint["name"]: joint["head"] for joint in definitions}
    bottom = heads["mPelvis"].z - 0.025
    shoulder = heads["mShoulderLeft"].z
    underarm, top = shoulder - 0.13, shoulder + 0.065
    vertices, faces, uv_faces, weights = [], [], [], []
    panels = []

    def face(indices, uvs):
        faces.append(indices)
        uv_faces.append(uvs)

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
                # Round the chest without pinching the open neck or side seams.
                depth *= 1 - 0.14 * min(1, (abs(y) / 0.24) ** 2) * (1 - sleeve)
                x = depth if front else -depth
                panel[row, col] = len(vertices)
                vertices.append(basis @ Vector((x, y, z)))
                weights.append(_weights(y, z, heads))
        for (row, col), index in panel.items():
            keys = ((row, col), (row, col + 1), (row + 1, col + 1), (row + 1, col))
            if all(key in panel for key in keys):
                face([panel[key] for key in keys],
                     [((key[1] + 16) / 64 + (0 if front else 0.5), key[0] / 10) for key in keys])
        panels.append(panel)

    front, back = panels
    seams = []
    for side in (-1, 1):
        seams.extend([((row, side * 8), (row + 1, side * 8)) for row in range(6)])
        columns = range(8, 16) if side == 1 else range(-16, -8)
        seams.extend([((6, col), (6, col + 1)) for col in columns])
        columns = range(3, 16) if side == 1 else range(-16, -3)
        seams.extend([((10, col), (10, col + 1)) for col in columns])
    for a, b in seams:
        face([front[a], front[b], back[b], back[a]], [(0, 0), (1, 0), (1, 1), (0, 1)])

    data = bpy.data.meshes.new("SL Test T-shirt")
    data.from_pydata(vertices, [], faces)
    data.update()
    uv = data.uv_layers.new(name="UVMap")
    for polygon, coords in zip(data.polygons, uv_faces):
        for loop, coord in zip(polygon.loop_indices, coords):
            uv.data[loop].uv = coord
    bm = bmesh.new()
    try:
        bm.from_mesh(data)
        bmesh.ops.recalc_face_normals(bm, faces=list(bm.faces))
        bm.to_mesh(data)
    finally:
        bm.free()
    mesh = bpy.data.objects.new("SL Test T-shirt", data)
    collection.objects.link(mesh)
    mesh.parent = rig
    mesh["aurora_starter"] = "Rigged test T-shirt"
    groups = {}
    for index, influences in enumerate(weights):
        for name, weight in influences:
            group = groups.get(name)
            if group is None:
                group = groups[name] = mesh.vertex_groups.new(name=name)
            group.add([index], weight, "REPLACE")
    modifier = mesh.modifiers.new("SL Avatar Rig", "ARMATURE")
    modifier.object = rig
    material = bpy.data.materials.new("Aurora Test Fabric")
    data.materials.append(material)
    material.diffuse_color = (0.34, 0.12, 0.72, 1)
    material.use_nodes = True
    # Blender localizes the default node's name when "Translate New Data" is on.
    shader = next((node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED"), None)
    if shader:
        shader.inputs["Base Color"].default_value = material.diffuse_color
        shader.inputs["Roughness"].default_value = 0.8
    for polygon in data.polygons:
        polygon.use_smooth = True
    return mesh


def create(context, with_tshirt=True, forward=None, definitions=None, collection_name="SL Avatar Starter"):
    """Create a new collection at the cursor; never reuse or overwrite a rig."""
    definitions = definitions if definitions is not None else joints()
    forward = forward or context.scene.aurora_live.forward
    if forward == "AUTO":
        forward = "NEGATIVE_Y"
    basis = _basis(context, forward)
    selected = list(context.selected_objects)
    active = context.view_layer.objects.active
    collection = bpy.data.collections.new(collection_name)
    context.scene.collection.children.link(collection)
    for obj in selected:
        obj.select_set(False)
    try:
        rig = _rig(context, collection, definitions, basis)
        rig["aurora_forward"] = forward
        shirt = _tshirt(collection, rig, definitions, basis) if with_tshirt else None
        if shirt:
            shirt.select_set(True)
        context.view_layer.objects.active = shirt or rig
        context.view_layer.update()
        return rig, shirt
    except Exception:
        # A failed operator must not leave half a starter in the user's scene.
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        for obj in list(collection.objects):
            data = obj.data
            materials = list(data.materials) if obj.type == "MESH" else []
            bpy.data.objects.remove(obj, do_unlink=True)
            if data.users == 0:
                (bpy.data.meshes if isinstance(data, bpy.types.Mesh) else bpy.data.armatures).remove(data)
            for material in materials:
                if material.users == 0:
                    bpy.data.materials.remove(material)
        bpy.data.collections.remove(collection)
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        raise
