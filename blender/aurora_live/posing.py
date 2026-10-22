"""Optional Blender-only root/COG and eye targets; never SL deform joints.

Avastar's animation and Eye Rig manuals describe the control/deform distinction
and the two independent eye pairs. These are Aurora controls, not Avastar data.
SL's runtime mRoot/LookAtPoint (llheadrotmotion.cpp) are viewer mechanisms;
the helpers below do not add network joints or stream Blender animation.
"""
from contextlib import contextmanager
import math
import bpy
from mathutils import Matrix, Vector
from . import rigging

ROOT, COG = "auroraPose.Root", "auroraPose.COG"
ROOT_CONSTRAINT = "Aurora pose root"
EYES = {
    "CLASSIC": ("Classic eyes", "mEyeLeft", "mEyeRight"),
    "BENTO": ("Bento eyes", "mFaceEyeAltLeft", "mFaceEyeAltRight"),
}


def target(key):
    return "auroraPose.Look." + key


def eye_constraint(key):
    return "Aurora look " + key


def joints(rig, names):
    lookup = {str(b.get("aurora_joint", b.name)): b.name for b in rig.data.bones if not b.get("aurora_control")}
    if any(name not in lookup for name in names):
        raise ValueError("This rig is missing the required SL joints")
    return [lookup[name] for name in names]


def root_ready(rig):
    if not all(name in rig.data.bones and rig.data.bones[name].get("aurora_control_kind") == "ROOT" and
               rig.data.bones[name].get("aurora_control") for name in (ROOT, COG)):
        return False
    pelvis = joints(rig, ["mPelvis"])[0]
    con = rig.pose.bones[pelvis].constraints.get(ROOT_CONSTRAINT)
    return con is not None and con.type == "CHILD_OF" and con.target == rig and con.subtarget == COG


def look_ready(rig, key):
    bone = rig.data.bones.get(target(key))
    if bone is None or not bone.get("aurora_control") or bone.get("aurora_control_kind") != "LOOK":
        return False
    names = joints(rig, EYES[key][1:])
    return all((con := rig.pose.bones[name].constraints.get(eye_constraint(key))) is not None and
               con.type == "DAMPED_TRACK" and con.target == rig and con.subtarget == target(key) for name in names)


@contextmanager
def editing(context, rig):
    rigging.editable(rig)
    if rig.mode == "EDIT":
        raise ValueError("Leave Edit Mode before creating pose controls")
    selected, active = list(context.selected_objects), context.view_layer.objects.active
    mode = active.mode if active else "OBJECT"
    try:
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        for obj in context.selected_objects:
            obj.select_set(False)
        rig.select_set(True)
        context.view_layer.objects.active = rig
        if rig.data.users > 1:
            rig.data = rig.data.copy()
        bpy.ops.object.mode_set(mode="EDIT")
        yield
    finally:
        if context.object and context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        for obj in context.selected_objects:
            obj.select_set(False)
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        if active and mode != "OBJECT":
            bpy.ops.object.mode_set(mode=mode)


def _preflight(rig, names, driven):
    rigging.editable(rig)
    if any(name in rig.data.bones for name in names):
        raise ValueError("Pose control names are already used; keep the existing rig unchanged")
    if any(rig.pose.bones[name].constraints for name in driven):
        raise ValueError("These joints already have constraints; use a simple SL rig for Aurora pose controls")


def _bone(rig, name, head, size, kind, parent=None):
    bone = rig.data.edit_bones.new(name)
    bone.head, bone.tail = head, head + Vector((0, size, 0))
    bone.parent = parent
    bone.use_deform = False
    bone["aurora_control"], bone["aurora_control_kind"] = True, kind
    return bone


def _group(rig, names, kind):
    label = "Aurora Root" if kind == "ROOT" else "Aurora Look"
    if hasattr(rig.data, "collections"):
        group = rig.data.collections.get(label) or rig.data.collections.new(label)
        group.is_visible = True
        for name in names:
            group.assign(rig.data.bones[name])
    else:
        index = 29 if kind == "ROOT" else 30
        group = rig.pose.bone_groups.get(label) or rig.pose.bone_groups.new(name=label)
        mask = list(rig.data.layers)
        mask[index] = True
        rig.data.layers = mask
        for name in names:
            rig.data.bones[name].layers = tuple(i == index for i in range(32))
            rig.pose.bones[name].bone_group = group


def _shape(rig, name, size):
    key = "Root" if name == ROOT else "COG" if name == COG else "Look"
    shape_name = "_AuroraPoseShape." + key
    obj = next((obj for obj in bpy.data.objects if obj.name == shape_name and obj.get("aurora_pose_shape")), None)
    if obj is None:
        if key == "Root":
            vertices = [(math.cos(i * math.tau / 24), math.sin(i * math.tau / 24), 0) for i in range(24)]
            edges = [(i, (i + 1) % 24) for i in range(24)]
        elif key == "COG":
            vertices = [(1, 0, 0), (0, 1, 0), (-1, 0, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)]
            edges = [(i, (i + 1) % 4) for i in range(4)] + [(i, j) for i in range(4) for j in (4, 5)]
        else:
            vertices = [(-1, 0, 0), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)]
            edges = [(0, 1), (2, 3), (4, 5)]
        mesh = bpy.data.meshes.new(shape_name)
        mesh.from_pydata(vertices, edges, [])
        obj = bpy.data.objects.new(shape_name, mesh)
        obj["aurora_pose_shape"] = True
        # Unlinked shape datablocks never become synchronized scene meshes.
    pose = rig.pose.bones[name]
    pose.custom_shape = obj
    pose.use_custom_shape_bone_size = False
    if hasattr(pose, "custom_shape_scale_xyz"):
        pose.custom_shape_scale_xyz = (size,) * 3
    else:
        pose.custom_shape_scale = size


def setup_root(context, rig):
    if root_ready(rig):
        return
    pelvis = joints(rig, ["mPelvis"])[0]
    _preflight(rig, (ROOT, COG), (pelvis,))
    center = rig.data.bones[pelvis].head_local.copy()
    size = max(rig.data.bones[pelvis].length * 2, 0.001)
    with editing(context, rig):
        root = _bone(rig, ROOT, Vector(), size, "ROOT")
        _bone(rig, COG, center, size, "ROOT", root)
        # Only our existing helper bones follow Root. SL parents/rest stay intact.
        for bone in rig.data.edit_bones:
            if bone.get("aurora_control") and bone.name not in {ROOT, COG} and bone.parent is None:
                bone.parent = root
    con = rig.pose.bones[pelvis].constraints.new("CHILD_OF")
    con.name, con.target, con.subtarget = ROOT_CONSTRAINT, rig, COG
    con.owner_space = con.target_space = "POSE"
    con.inverse_matrix = rig.pose.bones[COG].matrix.inverted()
    con.set_inverse_pending = False
    _group(rig, (ROOT, COG), "ROOT")
    _shape(rig, ROOT, size * 2.5)
    _shape(rig, COG, size * 0.7)
    context.view_layer.update()


def setup_look(context, rig, key):
    if look_ready(rig, key):
        return
    names = joints(rig, EYES[key][1:])
    _preflight(rig, (target(key),), names)
    eyes = [rig.data.bones[name] for name in names]
    size = max(sum(b.length for b in eyes), 0.001)
    position = sum((b.head_local for b in eyes), Vector()) / 2
    forward = sum((b.tail_local - b.head_local for b in eyes), Vector())
    if forward.length < 1e-6:
        raise ValueError("The eye pair has no common forward direction")
    forward.normalize()
    with editing(context, rig):
        root = rig.data.edit_bones.get(ROOT)
        if root and root.get("aurora_control_kind") != "ROOT":
            root = None
        _bone(rig, target(key), position + forward * size * 40, size, "LOOK", root)
    for name in names:
        con = rig.pose.bones[name].constraints.new("DAMPED_TRACK")
        con.name, con.target, con.subtarget = eye_constraint(key), rig, target(key)
        con.track_axis, con.influence = "TRACK_Y", 0
    _group(rig, (target(key),), "LOOK")
    _shape(rig, target(key), size)
    reset_look(context, rig, key)


def look_enabled(rig, key):
    names = joints(rig, EYES[key][1:])
    return all(rig.pose.bones[name].constraints[eye_constraint(key)].influence > 0.5 for name in names)


def set_look(context, rig, key, enabled):
    rigging.editable(rig)
    if rig.mode == "EDIT" or rig.data.pose_position != "POSE":
        raise ValueError("Use Pose Position and leave Edit Mode to change eye focus")
    if not look_ready(rig, key):
        raise ValueError("Create the eye target first")
    context.view_layer.update()
    eyes = [rig.pose.bones[name] for name in joints(rig, EYES[key][1:])]
    matrices = [bone.matrix.copy() for bone in eyes]
    for bone in eyes:
        bone.constraints[eye_constraint(key)].influence = float(enabled)
    context.view_layer.update()
    if not enabled:
        for bone, matrix in zip(eyes, matrices):
            bone.matrix = matrix
        context.view_layer.update()


def reset_look(context, rig, key):
    rigging.editable(rig)
    if not look_ready(rig, key):
        raise ValueError("Create the eye target first")
    if rig.mode == "EDIT":
        raise ValueError("Leave Edit Mode before creating pose controls")
    eyes = [rig.data.bones[name] for name in joints(rig, EYES[key][1:])]
    size = max(sum(b.length for b in eyes), 0.001)
    position = sum((b.head_local for b in eyes), Vector()) / 2
    forward = sum((b.tail_local - b.head_local for b in eyes), Vector()).normalized()
    parent = rig.pose.bones[eyes[0].parent.name] if eyes[0].parent else None
    transform = parent.matrix @ parent.bone.matrix_local.inverted() if parent else Matrix.Identity(4)
    matrix = rig.pose.bones[target(key)].matrix.copy()
    matrix.translation = transform @ (position + forward * size * 40)
    rig.pose.bones[target(key)].matrix = matrix
    context.view_layer.update()
