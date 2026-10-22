"""SL/Bento visibility and optional non-deforming hand/foot IK controls."""
import math
import bpy
from mathutils import Matrix, Vector

PARTS = (
    ("BODY", "Body"), ("ARMS", "Arms"), ("LEGS", "Legs"), ("HANDS", "Bento fingers"),
    ("FACE", "Bento face / eyes"), ("WINGS", "Bento wings"), ("TAIL", "Bento tail"),
    ("HIND", "Bento hind limbs"), ("FITTED", "Fitted Mesh volumes"), ("IK", "IK controls"), ("EXTRA", "Other bones"),
)
LIMBS = {
    "HAND_L": ("Left hand IK", "mShoulderLeft", "mElbowLeft", "mWristLeft"),
    "HAND_R": ("Right hand IK", "mShoulderRight", "mElbowRight", "mWristRight"),
    "FOOT_L": ("Left foot IK", "mHipLeft", "mKneeLeft", "mAnkleLeft"),
    "FOOT_R": ("Right foot IK", "mHipRight", "mKneeRight", "mAnkleRight"),
}


def resolve(context):
    rig = context.scene.aurora_live.rig
    if rig is None:
        obj = context.object
        rig = obj if obj and obj.type == "ARMATURE" else obj.find_armature() if obj and obj.type == "MESH" else None
    if rig is None or rig.type != "ARMATURE":
        raise ValueError("Choose an avatar rig first")
    return rig


def editable(rig):
    if rig.library or rig.data.library:
        raise ValueError("Use an editable local armature")


def bones(rig):
    return rig.data.edit_bones if rig.mode == "EDIT" else rig.data.bones


def part(bone):
    name = str(bone.get("aurora_joint", bone.name))
    if bone.get("aurora_control", False):
        return "IK"
    if bone.get("sl_collision_volume", False) or name.isupper():
        return "FITTED"
    for prefix, result in (("mHand", "HANDS"), ("mFace", "FACE"), ("mEye", "FACE"),
                           ("mWing", "WINGS"), ("mTail", "TAIL"), ("mHind", "HIND")):
        if name.startswith(prefix):
            return result
    if any(name.startswith("m" + prefix) for prefix in ("Collar", "Shoulder", "Elbow", "Wrist")):
        return "ARMS"
    if any(name.startswith("m" + prefix) for prefix in ("Hip", "Knee", "Ankle", "Foot", "Toe")):
        return "LEGS"
    return "BODY" if name.startswith(("mPelvis", "mTorso", "mChest", "mSpine", "mNeck", "mHead", "mSkull")) else "EXTRA"


def _display_state(rig, bone):
    # Blender 5 stores object/pose visibility per PoseBone; Bone.hide now
    # controls Edit Mode only. Older Blender keeps pose visibility on Bone.
    if rig.mode != "EDIT":
        pose_bone = rig.pose.bones.get(bone.name)
        if pose_bone is not None and hasattr(pose_bone, "hide"):
            return pose_bone
    return bone


def visible(rig, bone):
    if _display_state(rig, bone).hide:
        return False
    if hasattr(bone, "collections"):
        return not bone.collections or any(getattr(group, "is_visible_effectively", group.is_visible) for group in bone.collections)
    return any(a and b for a, b in zip(bone.layers, rig.data.layers))


def part_visible(rig, key):
    return any(visible(rig, bone) for bone in bones(rig) if part(bone) == key)


def _select(rig, bone, enabled):
    # Blender 5 moved pose selection from Bone to PoseBone.
    if hasattr(bone, "select"):
        bone.select = enabled
    else:
        rig.pose.bones[bone.name].select = enabled


def show(rig, key, enabled):
    editable(rig)
    items = list(bones(rig))
    targets = [bone for bone in items if key == "ALL" or (key == "DEFORM" and bone.use_deform) or part(bone) == key]
    old = {bone.name: visible(rig, bone) for bone in items}
    if enabled:
        if hasattr(rig.data, "collections"):
            for group in rig.data.collections_all if hasattr(rig.data, "collections_all") else rig.data.collections:
                if hasattr(group, "is_solo"):
                    group.is_solo = False
            for bone in targets:
                for group in bone.collections:
                    group.is_visible = True
                    parent = getattr(group, "parent", None)
                    while parent:
                        parent.is_visible = True
                        parent = parent.parent
        else:
            mask = list(rig.data.layers)
            for bone in targets:
                mask = [a or b for a, b in zip(mask, bone.layers)]
            rig.data.layers = mask
        # Enabling a shared collection/layer must not reveal unrelated parts.
        for bone in items:
            _display_state(rig, bone).hide = not old[bone.name]
    for bone in targets:
        _display_state(rig, bone).hide = not enabled
        if not enabled:
            _select(rig, bone, False)


def control_name(key, pole=False):
    return "auroraIK." + ("Pole." if pole else "Target.") + key


def constraint_name(key):
    return "Aurora IK " + key


def chain(rig, key):
    lookup = {str(bone.get("aurora_joint", bone.name)): bone.name for bone in rig.data.bones if not bone.get("aurora_control", False)}
    try:
        return tuple(lookup[name] for name in LIMBS[key][1:])
    except KeyError:
        raise ValueError("This armature is missing the SL arm or leg chain") from None


def constraint(rig, key):
    try:
        return rig.pose.bones[chain(rig, key)[1]].constraints.get(constraint_name(key))
    except ValueError:
        return None


def ready(rig):
    return all(constraint(rig, key) and all(control_name(key, pole) in rig.data.bones for pole in (False, True)) for key in LIMBS)


def _pole(root, middle, end):
    axis = end - root
    if axis.length < 1e-6:
        raise ValueError("The IK chain has zero length")
    direction = axis.normalized()
    perpendicular = middle - root - direction * (middle - root).dot(direction)
    if perpendicular.length < axis.length * 0.001:
        fallback = Vector((-1, 0, 0))
        if abs(fallback.dot(direction)) > 0.9:
            fallback = Vector((0, 1, 0))
        perpendicular = fallback - direction * fallback.dot(direction)
    return middle + perpendicular.normalized() * axis.length * 0.75


def setup(context, rig):
    editable(rig)
    if ready(rig):
        return
    if rig.mode == "EDIT":
        raise ValueError("Leave Edit Mode before creating IK controls")
    chains = {key: chain(rig, key) for key in LIMBS}
    for key, names in chains.items():
        for name in names:
            if any(not c.name.startswith("Aurora IK ") for c in rig.pose.bones[name].constraints):
                raise ValueError("The limb already has constraints; use a simple SL rig for Aurora IK")
        for pole in (False, True):
            bone = rig.data.bones.get(control_name(key, pole))
            if bone and not bone.get("aurora_control", False):
                raise ValueError("An IK control name is already used by another bone")
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
        for key, names in chains.items():
            root, middle, end = (rig.data.edit_bones[name] for name in names)
            end_matrix, end_length, tip = end.matrix.copy(), end.length, middle.tail.copy()
            pole_position = _pole(root.head, middle.head, tip)
            size = max((middle.head - root.head).length * 0.3, 0.001)
            for pole in (False, True):
                name = control_name(key, pole)
                if name in rig.data.edit_bones:
                    continue
                bone = rig.data.edit_bones.new(name)
                bone.matrix = end_matrix
                bone.length = max(end_length, size)
                bone.head = pole_position if pole else tip
                bone.tail = bone.head + (end_matrix.to_3x3() @ Vector((0, size, 0)))
                bone.use_deform = False
                bone["aurora_control"] = True
        bpy.ops.object.mode_set(mode="OBJECT")
        if hasattr(rig.data, "collections"):
            group = rig.data.collections.get("Aurora IK") or rig.data.collections.new("Aurora IK")
            group.is_visible = True
            for key in LIMBS:
                for pole in (False, True):
                    group.assign(rig.data.bones[control_name(key, pole)])
        else:
            groups = rig.pose.bone_groups
            group = groups.get("Aurora IK") or groups.new(name="Aurora IK")
            mask = list(rig.data.layers)
            mask[31] = True
            rig.data.layers = mask
            for key in LIMBS:
                for pole in (False, True):
                    name = control_name(key, pole)
                    rig.data.bones[name].layers = tuple(index == 31 for index in range(32))
                    rig.pose.bones[name].bone_group = group
        for key, names in chains.items():
            middle, end = rig.pose.bones[names[1]], rig.pose.bones[names[2]]
            ik = middle.constraints.get(constraint_name(key)) or middle.constraints.new("IK")
            ik.name = constraint_name(key)
            ik.target, ik.subtarget = rig, control_name(key)
            ik.pole_target, ik.pole_subtarget = rig, control_name(key, True)
            ik.chain_count, ik.use_tail, ik.use_stretch, ik.iterations = 2, True, False, 128
            ik.influence = 0
            rotation = end.constraints.get(constraint_name(key)) or end.constraints.new("COPY_ROTATION")
            rotation.name = constraint_name(key)
            rotation.target, rotation.subtarget = rig, control_name(key)
            rotation.owner_space = rotation.target_space = "POSE"
            rotation.influence = 0
        rig["aurora_ik_version"] = 1
        context.view_layer.update()
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


def set_ik(context, rig, key, enabled):
    editable(rig)
    if not ready(rig):
        raise ValueError("Create Aurora IK controls first")
    if rig.data.pose_position != "POSE" or rig.mode == "EDIT":
        raise ValueError("Use Pose Position and leave Edit Mode to switch IK")
    context.view_layer.update()
    names = chain(rig, key)
    root, middle, end = (rig.pose.bones[name] for name in names)
    ik = constraint(rig, key)
    rotation = end.constraints[constraint_name(key)]
    matrices = [bone.matrix.copy() for bone in (root, middle, end)]
    if enabled:
        target = matrices[2].copy()
        target.translation = middle.tail.copy()
        rig.pose.bones[control_name(key)].matrix = target
        pole_position = _pole(root.head.copy(), middle.head.copy(), middle.tail.copy())
        pole = rig.pose.bones[control_name(key, True)]
        matrix = pole.matrix.copy()
        matrix.translation = pole_position
        pole.matrix = matrix
        direction = (middle.tail - root.head).cross(pole_position - root.head).cross(root.tail - root.head)
        axis = (root.tail - root.head).normalized()
        if direction.length > 1e-6:
            x = root.matrix.to_3x3().col[0].normalized()
            y = direction.normalized()
            ik.pole_angle = -math.atan2(axis.dot(x.cross(y)), x.dot(y))
        upper, lower = root.tail - root.head, middle.tail - middle.head
        if upper.normalized().cross(lower.normalized()).length < 0.0001:
            # SL's straight T-pose is a singular IK seed. A tiny local pose
            # bend lets Blender solve subsequent target moves. Rest bones and
            # the snapped endpoint stay unchanged; disabling bakes the result.
            middle.matrix_basis = middle.matrix_basis @ Matrix.Rotation(0.001, 4, "X")
        ik.influence = rotation.influence = 1
    else:
        ik.influence = rotation.influence = 0
        context.view_layer.update()
        for bone, matrix in zip((root, middle, end), matrices):
            bone.matrix = matrix
            context.view_layer.update()
    context.view_layer.update()


def select_control(context, rig, key, pole=False):
    if not ready(rig):
        raise ValueError("Create Aurora IK controls first")
    if context.object and context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for obj in context.selected_objects:
        obj.select_set(False)
    rig.hide_set(False)
    rig.select_set(True)
    context.view_layer.objects.active = rig
    show(rig, "IK", True)
    for bone in rig.data.bones:
        _select(rig, bone, False)
    bone = rig.data.bones[control_name(key, pole)]
    _select(rig, bone, True)
    rig.data.bones.active = bone
    bpy.ops.object.mode_set(mode="POSE")
