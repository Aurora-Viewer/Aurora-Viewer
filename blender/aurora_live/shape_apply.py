"""Apply shape data to a selected SL rig without replacing its objects or weights.

Generic rigs receive skeletal proportions, with bound geometry and shape keys
following the rest change. Avastar rigs use their own slider updater, which also
handles their body morphs and control-bone bookkeeping. No vendor code is bundled.
"""
import json
import sys

import bpy
import numpy as np
from mathutils import Matrix, Vector
from . import exporter, rigging


def _matrix(values):
    if len(values) != 16 or not np.isfinite(values).all():
        raise ValueError("Invalid previous shape bind")
    result = Matrix([values[i:i+4] for i in range(0, 16, 4)]).transposed()
    if abs(result.determinant()) < 1e-8:
        raise ValueError("Invalid previous shape bind")
    return result


def _remember(rig, payload, mapping):
    rig["aurora_shape_id"] = payload["id"]
    rig["aurora_shape_parameters"] = json.dumps(payload["params"], sort_keys=True)
    for joint in payload["joints"]:
        bone = mapping.get(joint["name"])
        if bone is not None:
            bone["aurora_shape_bind"] = joint["inverse_bind"]
            bone["aurora_shape_head"] = joint["head"]


def _avastar(context, rig, payload, mapping):
    # Use the already enabled add-on's own dictionary/refresh API. Its driver
    # definitions provide SL min/max ranges; its slider values use 0..100.
    module = next((module for name, module in tuple(sys.modules.items())
                   if name.endswith("avastar.shape") and all(callable(getattr(module, function, None)) for function in
                   ("fromDictionary", "asDictionary", "ensure_drivers_initialized"))), None)
    if module is None or not hasattr(rig, "ShapeDrivers"):
        raise ValueError("Enable Avastar before applying a shape to an Avastar rig")
    if not getattr(getattr(context.scene, "SceneProp", None), "panel_appearance_enabled", True):
        raise ValueError("Enable Avastar appearance sliders before applying the shape")
    module.ensure_drivers_initialized(rig)
    values = {}
    for pid, drivers in rig.ShapeDrivers.DRIVERS.items():
        value = payload["params"].get(pid.rsplit("_", 1)[-1])
        if value is None or not drivers:
            continue
        definition = drivers[0]
        low, high = definition["value_min"], definition["value_max"]
        if high > low:
            values[pid] = (value - low) * 100 / (high - low) if pid != "male_80" else float(value > 0)
    if not values:
        raise ValueError("This Avastar version has no compatible shape drivers")
    previous = module.asDictionary(rig, full=True)
    bone_names = {name: bone.name for name, bone in mapping.items()}
    try:
        module.fromDictionary(rig, values, update=True)
    except Exception:
        module.fromDictionary(rig, previous, update=True)
        raise
    # Its updater enters Edit Mode, invalidating the old Bone RNA wrappers.
    # Resolve fresh wrappers after refresh before writing bind metadata.
    _remember(rig, payload, {name: rig.data.bones[bone] for name, bone in bone_names.items()})
    context.view_layer.update()


def _geometry(obj, rig, deltas):
    modifiers = [mod for mod in obj.modifiers if mod.type == "ARMATURE" and mod.object == rig]
    if len(modifiers) != 1 or modifiers[0].use_bone_envelopes or modifiers[0].vertex_group:
        raise ValueError("Shape application requires one unmasked vertex-group Armature modifier: " + obj.name)
    if obj.library or obj.data.library:
        raise ValueError("Use editable local bound meshes: " + obj.name)
    transform = rig.matrix_world.inverted() @ obj.matrix_world
    inverse = transform.inverted()
    groups = {group.index: np.array(inverse @ deltas[group.name] @ transform, dtype=np.float64)
              for group in obj.vertex_groups if group.name in deltas and rig.data.bones[group.name].use_deform}
    # Build one affine blend per vertex, then apply it equally to the Basis and
    # every authored shape key. All weights (including non-bone groups) survive.
    matrices = np.tile(np.eye(4), (len(obj.data.vertices), 1, 1))
    for vertex in obj.data.vertices:
        influences = [(groups[group.group], group.weight) for group in vertex.groups
                      if group.group in groups and group.weight > 0]
        total = sum(weight for _, weight in influences)
        if total > 1e-8:
            matrices[vertex.index] = sum(matrix * (weight / total) for matrix, weight in influences)
    clone = obj.data.copy()
    try:
        blocks = [clone.vertices] if clone.shape_keys is None else [block.data for block in clone.shape_keys.key_blocks]
        for block in blocks:
            co = np.empty(len(block) * 3, dtype=np.float64)
            block.foreach_get("co", co)
            transformed = np.einsum("nij,nj->ni", matrices[:, :3, :3], co.reshape((-1, 3))) + matrices[:, :3, 3]
            if not np.isfinite(transformed).all():
                raise ValueError("Invalid transformed shape geometry")
            block.foreach_set("co", transformed.ravel())
        if clone.shape_keys:
            clone.vertices.foreach_set("co", np.array([point.co[:] for point in clone.shape_keys.reference_key.data]).ravel())
        clone.update()
        return clone
    except Exception:
        bpy.data.meshes.remove(clone)
        raise


def apply(context, payload):
    rig = context.view_layer.objects.active
    if rig is None or rig.type != "ARMATURE" or not rig.select_get():
        raise ValueError("Select the target SL skeleton in Blender first")
    rigging.editable(rig)
    mapping = {}
    names = set(payload["names"])
    for bone in rig.data.bones:
        name = str(bone.get("aurora_joint", bone.name))
        if name in names and not bone.get("aurora_control"):
            if name in mapping:
                raise ValueError("Multiple bones map to the same SL joint: " + name)
            mapping[name] = bone
    if not {"mPelvis", "mTorso", "mChest", "mHead"}.issubset(mapping):
        raise ValueError("The selected skeleton is not a compatible Second Life rig")
    if rig.get("aurora_shape_id") == payload["id"]:
        return rig
    if "avastar" in rig:
        _avastar(context, rig, payload, mapping)
        context.scene.aurora_live.rig = rig
        return rig

    basis = exporter.coordinate_basis(rig) @ Matrix.Scale(context.scene.unit_settings.scale_length, 4)
    inverse = basis.inverted()
    deltas = {}
    for joint in payload["joints"]:
        bone = mapping.get(joint["name"])
        if bone is not None:
            previous = bone.get("aurora_shape_bind") if rig.get("aurora_shape_id") else joint["reference_bind"]
            deltas[bone.name] = inverse @ _matrix(joint["inverse_bind"]).inverted() @ _matrix(previous) @ basis
    # Keep Aurora's existing control offsets, hierarchy and constraints. The
    # ground root stays anchored; targets follow the corresponding end joint.
    for bone in rig.data.bones:
        if bone.name in deltas:
            continue
        joint = "m" + bone.name
        if bone.name == "auroraPose.COG":
            joint = "mPelvis"
        for key, chain in rigging.LIMBS.items():
            if bone.name == rigging.control_name(key):
                joint = chain[3]
            elif bone.name == rigging.control_name(key, True):
                joint = chain[2]
        if bone.name.startswith("auroraPose.Look."):
            joint = "mHead"
        match = mapping.get(joint)
        deltas[bone.name] = deltas.get(match.name if match else bone.parent.name if bone.parent else "", Matrix.Identity(4))

    selected, active = list(context.selected_objects), context.view_layer.objects.active
    original, staged, old_meshes = rig.data, {}, {}
    clone, temporary = None, None
    try:
        # Prepare disposable data before replacing anything on the user's rig.
        for obj in context.scene.objects:
            if obj.type == "MESH" and any(mod.type == "ARMATURE" and mod.object == rig for mod in obj.modifiers):
                staged[obj] = _geometry(obj, rig, deltas)
        clone = original.copy()
        temporary = bpy.data.objects.new("_AuroraShapeRest", clone)
        context.scene.collection.objects.link(temporary)
        for obj in selected:
            obj.select_set(False)
        temporary.select_set(True)
        context.view_layer.objects.active = temporary
        bpy.ops.object.mode_set(mode="EDIT")
        connected = {bone.name: bone.use_connect for bone in clone.edit_bones}
        for bone in clone.edit_bones:
            bone.use_connect = False
        for bone in clone.edit_bones:
            bone.transform(deltas[bone.name], scale=True, roll=True)
        for bone in clone.edit_bones:
            bone.use_connect = connected[bone.name] and bone.parent is not None and (bone.head - bone.parent.tail).length < 1e-5
        bpy.ops.object.mode_set(mode="OBJECT")
        rig.data = clone
        for obj, mesh in staged.items():
            old_meshes[obj] = obj.data
            obj.data = mesh
        # A Child Of inverse encodes the control's old rest position. Preserve
        # the authored pose, but cancel the new COG rest position exactly once.
        if "auroraPose.COG" in clone.bones:
            correction = original.bones["auroraPose.COG"].matrix_local @ clone.bones["auroraPose.COG"].matrix_local.inverted()
            for bone in rig.pose.bones:
                con = bone.constraints.get("Aurora pose root")
                if con and con.type == "CHILD_OF" and con.target == rig and con.subtarget == "auroraPose.COG":
                    con.inverse_matrix = con.inverse_matrix @ correction
        _remember(rig, payload, {name: rig.data.bones[bone.name] for name, bone in mapping.items()})
        context.scene.aurora_live.rig = rig
        context.view_layer.update()
        return rig
    except Exception:
        rig.data = original
        for obj, mesh in old_meshes.items():
            obj.data = mesh
        raise
    finally:
        if temporary:
            if temporary.mode != "OBJECT":
                bpy.ops.object.mode_set(mode="OBJECT")
            bpy.data.objects.remove(temporary, do_unlink=True)
        for obj in context.selected_objects:
            obj.select_set(False)
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        for data in staged.values():
            if data.users == 0:
                bpy.data.meshes.remove(data)
        if clone and clone.users == 0:
            bpy.data.armatures.remove(clone)
