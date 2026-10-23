"""Receive an authorized SL shape as a new reference or on a selected SL rig.

Only Blender's main thread calls this module. Payloads contain plain bounded
data; no script, path, texture or existing object is accepted from the viewer.
"""
import json
import math
import uuid

import bpy
from mathutils import Matrix, Quaternion, Vector
from . import starter, shape_apply


def _numbers(value, length):
    if not isinstance(value, (list, tuple)) or len(value) != length:
        raise ValueError("Invalid shape vector")
    if any(type(number) not in (int, float) or not math.isfinite(number) or abs(number) > 10000 for number in value):
        raise ValueError("Invalid shape number")
    return value


def validate(payload):
    if not isinstance(payload, dict):
        raise ValueError("Invalid shape transfer")
    if payload.get("action", "create") not in {"create", "apply"}:
        raise ValueError("Unsupported shape action")
    uuid.UUID(payload["id"])
    name = payload["name"]
    if not isinstance(name, str) or len(name) > 128:
        raise ValueError("Invalid shape name")
    definitions = starter.joints()
    names = [joint["name"] for joint in definitions]
    if payload["names"] != names or len(payload["joints"]) != len(names):
        raise ValueError("Incompatible SL shape skeleton")
    for definition, joint in zip(definitions, payload["joints"]):
        if joint["name"] != definition["name"]:
            raise ValueError("Incompatible SL shape joint")
        definition["head"] = Vector(_numbers(joint["head"], 3))
        definition["tail"] = Vector(_numbers(joint["tail"], 3))
        x, y, z, w = _numbers(joint["rotation"], 4)
        if abs(x*x + y*y + z*z + w*w - 1) > 0.001:
            raise ValueError("Invalid shape rotation")
        definition["rotation"] = Quaternion((w, x, y, z))
        values = _numbers(joint["inverse_bind"], 16)
        inverse = Matrix([values[i:i+4] for i in range(0, 16, 4)]).transposed()
        if abs(inverse.determinant()) < 1e-8 or (inverse @ definition["head"]).length > 0.0001:
            raise ValueError("Invalid shape bind matrix")
        if payload.get("action") == "apply":
            values = _numbers(joint["reference_bind"], 16)
            if abs(Matrix([values[i:i+4] for i in range(0, 16, 4)]).determinant()) < 1e-8:
                raise ValueError("Invalid reference shape bind")
    params = payload["params"]
    if not isinstance(params, dict) or len(params) > 512:
        raise ValueError("Invalid shape parameters")
    for key, value in params.items():
        if not str(key).isdigit():
            raise ValueError("Invalid shape parameter id")
        _numbers([value], 1)
    parts = payload["parts"]
    if not isinstance(parts, list) or not 1 <= len(parts) <= 6:
        raise ValueError("Invalid shape body parts")
    vertices = 0
    for part in parts:
        if part["name"] not in {"head", "upper", "lower", "eye_left", "eye_right", "eyelashes"}:
            raise ValueError("Invalid shape body part")
        count = len(part["positions"])
        vertices += count
        if not count or vertices > 50000 or len(part["uvs"]) != count or len(part["weights"]) != count:
            raise ValueError("Invalid shape body vertex count")
        for position, uv, groups in zip(part["positions"], part["uvs"], part["weights"]):
            _numbers(position, 3)
            _numbers(uv, 2)
            if not 1 <= len(groups) <= 4:
                raise ValueError("Invalid shape weights")
            total = 0.0
            for index, weight in groups:
                if type(index) is not int or not 0 <= index < len(names) or not 0 < _numbers([weight], 1)[0] <= 1:
                    raise ValueError("Invalid shape influence")
                total += weight
            if abs(total - 1) > 0.001:
                raise ValueError("Invalid shape weight sum")
        if not part["triangles"] or len(part["triangles"]) > 100000:
            raise ValueError("Invalid shape triangle count")
        for triangle in part["triangles"]:
            if len(triangle) != 3 or any(type(index) is not int or not 0 <= index < count for index in triangle):
                raise ValueError("Invalid shape triangle")
    return definitions


def apply(context, payload):
    definitions = validate(payload)
    if payload.get("action") == "apply":
        mode = context.object.mode if context.object else "OBJECT"
        if mode not in {"OBJECT", "POSE"}:
            raise ValueError("Leave Edit or Paint Mode to apply the shape")
        if mode == "POSE":
            bpy.ops.object.mode_set(mode="OBJECT")
        try:
            return shape_apply.apply(context, payload)
        finally:
            if mode == "POSE":
                bpy.ops.object.mode_set(mode="POSE")
    if context.object and context.object.mode != "OBJECT":
        raise ValueError("Switch to Object Mode to receive the shape")
    existing = next((obj for obj in context.scene.objects if obj.type == "ARMATURE"
                     and obj.get("aurora_shape_id") == payload["id"]), None)
    if existing:
        return existing
    selected, active = list(context.selected_objects), context.view_layer.objects.active
    previous_rig = context.scene.aurora_live.rig
    rig, _unused = starter.create(context, with_tshirt=False, definitions=definitions,
                                 collection_name="Aurora Shape · " + payload["name"])
    collection = rig.users_collection[0]
    try:
        rig.name = "Shape · " + payload["name"]
        rig["aurora_shape_id"] = payload["id"]
        rig["aurora_shape_parameters"] = json.dumps(payload["params"], sort_keys=True)
        basis = starter._basis(context, rig["aurora_forward"])
        for definition, joint in zip(definitions, payload["joints"]):
            bone = rig.data.bones[definition["name"]]
            # Canonical metres, independent of Blender roll and scene units.
            bone["aurora_shape_bind"] = joint["inverse_bind"]
            bone["aurora_shape_head"] = joint["head"]
        for part in payload["parts"]:
            data = bpy.data.meshes.new("Shape " + part["name"])
            obj = bpy.data.objects.new("Shape " + part["name"], data)
            collection.objects.link(obj)
            data.from_pydata([basis @ Vector(position) for position in part["positions"]], [], part["triangles"])
            data.update()
            uv = data.uv_layers.new(name="UVMap")
            for loop in data.loops:
                uv.data[loop.index].uv = part["uvs"][loop.vertex_index]
            groups = {}
            for index, influences in enumerate(part["weights"]):
                merged = {}
                for joint, weight in influences:
                    merged[joint] = merged.get(joint, 0) + weight
                for joint, weight in merged.items():
                    group = groups.get(joint)
                    if group is None:
                        group = groups[joint] = obj.vertex_groups.new(name=payload["names"][joint])
                    group.add([index], weight, "REPLACE")
            for polygon in data.polygons:
                polygon.use_smooth = True
            obj.parent = rig
            obj.location = (0, 0, 0)
            obj.modifiers.new("SL Avatar", "ARMATURE").object = rig
            obj["aurora_shape_reference"] = True
        context.scene.aurora_live.rig = rig
        context.view_layer.objects.active = rig
        context.view_layer.update()
        return rig
    except Exception:
        for obj in list(collection.objects):
            data = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if data.users == 0:
                (bpy.data.meshes if isinstance(data, bpy.types.Mesh) else bpy.data.armatures).remove(data)
        bpy.data.collections.remove(collection)
        context.scene.aurora_live.rig = previous_rig
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        raise
