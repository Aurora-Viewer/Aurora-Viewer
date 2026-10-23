"""SL/Bento visibility and optional non-deforming hand/foot IK controls."""
import math
import bpy
from mathutils import Vector
from . import handles

PARTS = (
    ("BACK", "Back / torso"), ("HEAD", "Head / neck"), ("ARMS", "Arms"), ("LEGS", "Legs"), ("HANDS", "Bento fingers"),
    ("FACE", "Bento face / eyes"), ("WINGS", "Bento wings"), ("TAIL", "Bento tail"),
    ("HIND", "Bento hind limbs"), ("FITTED", "Fitted Mesh volumes"), ("IK", "IK controls"), ("EXTRA", "Other bones"),
    ("SPINE", "Bento spine"), ("ROOT", "Root / COG controls"), ("LOOK", "Look-at targets"),
)
SPINE = frozenset({"mSpine1", "mSpine2", "mSpine3", "mSpine4"})
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
        return bone.get("aurora_control_kind", "IK")
    if bone.get("sl_collision_volume", False) or name.isupper():
        return "FITTED"
    if name in SPINE:
        return "SPINE"
    for prefix, result in (("mHand", "HANDS"), ("mFace", "FACE"), ("mEye", "FACE"),
                           ("mWing", "WINGS"), ("mTail", "TAIL"), ("mHind", "HIND")):
        if name.startswith(prefix):
            return result
    if any(name.startswith("m" + prefix) for prefix in ("Collar", "Shoulder", "Elbow", "Wrist")):
        return "ARMS"
    if any(name.startswith("m" + prefix) for prefix in ("Hip", "Knee", "Ankle", "Foot", "Toe")):
        return "LEGS"
    if name.startswith(("mPelvis", "mTorso", "mChest")):
        return "BACK"
    return "HEAD" if name.startswith(("mNeck", "mHead", "mSkull")) else "EXTRA"


def _belongs(bone, key):
    # Retain BODY for existing operator calls, now as the classic back + head.
    return part(bone) in {"BACK", "HEAD"} if key == "BODY" else part(bone) == key


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
    return any(visible(rig, bone) for bone in bones(rig) if _belongs(bone, key))


def _select(rig, bone, enabled):
    # Blender 5 moved pose selection from Bone to PoseBone.
    if hasattr(bone, "select"):
        bone.select = enabled
    else:
        rig.pose.bones[bone.name].select = enabled


def show(rig, key, enabled):
    editable(rig)
    items = list(bones(rig))
    targets = [bone for bone in items if key == "ALL" or (key == "DEFORM" and bone.use_deform) or _belongs(bone, key)]
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
    if key in {"BACK", "BODY"}:
        # SL's four extended spine joints double back inside the classic
        # spine. Keep them in the hierarchy and deform palette, but show a
        # simple back on every ordinary Back display action. Advanced display
        # can reveal them explicitly afterward; ALL/DEFORM still include them.
        for bone in items:
            if part(bone) == "SPINE":
                _display_state(rig, bone).hide = True
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


def _pole_settings(rig, key, posed=False):
    # Avatar facing comes from SL joint geometry, independent of bone roll,
    # object rotation and the starter's native/-Y orientation.
    left, right = (rig.data.bones[chain(rig, hand)[0]] for hand in ("HAND_L", "HAND_R"))
    pelvis = next((bone for bone in rig.data.bones if bone.get("aurora_joint", bone.name) == "mPelvis"), None)
    up = pelvis.tail_local - pelvis.head_local if pelvis else Vector((0, 0, 1))
    direction = (left.head_local - right.head_local).normalized().cross(up.normalized())
    if direction.length < 1e-6:
        raise ValueError("Cannot determine the avatar's facing direction")
    direction.normalize()
    hand = key.startswith("HAND")
    if hand:
        direction.negate()
    root, middle, _end = (rig.data.bones[name] for name in chain(rig, key))
    if posed:
        direction = rig.pose.bones[root.name].matrix.to_3x3() @ root.matrix_local.to_3x3().inverted() @ direction
    return direction, (middle.head_local - root.head_local).length * (2 if hand else 0.6)


def _pole(root, middle, end, direction, distance, follow_bend=True):
    axis = end - root
    if axis.length < 1e-6:
        raise ValueError("The IK chain has zero length")
    axis_direction = axis.normalized()
    perpendicular = middle - root - axis_direction * (middle - root).dot(axis_direction)
    # SL's nearly straight legs have a small sideways rest bend. Amplifying
    # it crosses the knee poles. Use anatomical defaults near extension;
    # a deliberately bent FK limb instead keeps its current bend plane.
    if follow_bend and perpendicular.length > axis.length * 0.05:
        direction = perpendicular
    if direction.cross(axis).length < direction.length * axis.length * 0.001:
        direction = perpendicular if perpendicular.length > axis.length * 0.001 else axis_direction.orthogonal()
    return middle + direction.normalized() * distance


def _aim(context, bone, direction):
    matrix = bone.matrix.copy()
    turn = (bone.tail - bone.head).rotation_difference(direction)
    aimed = turn.to_matrix().to_4x4() @ matrix
    aimed.translation = matrix.translation
    bone.matrix = aimed
    context.view_layer.update()


def _seed_bend(context, root, middle, tip, pole):
    # Solve the triangle in the pole plane before handing it to Blender.
    # Shortest-arc rotations retain bone twist; rest data never changes.
    upper, lower = (middle.head - root.head).length, (middle.tail - middle.head).length
    axis = tip - root.head
    if min(upper, lower, axis.length) < 1e-6:
        raise ValueError("The IK chain has zero length")
    gap = min((upper + lower) * 0.0005, min(upper, lower) * 0.1)
    distance = max(abs(upper - lower) + gap, min(axis.length, upper + lower - gap))
    axis.normalize()
    toward = pole - root.head
    toward -= axis * toward.dot(axis)
    if toward.length < (upper + lower) * 1e-6:
        toward = axis.orthogonal()
    toward.normalize()
    along = (upper * upper - lower * lower + distance * distance) / (2 * distance)
    height = math.sqrt(max(0, upper * upper - along * along))
    base = root.head.copy()
    joint = base + axis * along + toward * height
    _aim(context, root, joint - base)
    _aim(context, middle, base + axis * distance - middle.head)


def _align_pole(context, rig, key, reposition=True, preserve_pose=True):
    root, middle, _end = (rig.pose.bones[name] for name in chain(rig, key))
    pole = rig.pose.bones[control_name(key, True)]
    if reposition:
        position = _pole(root.head.copy(), middle.head.copy(), middle.tail.copy(), *_pole_settings(rig, key, posed=True))
        matrix = pole.matrix.copy()
        matrix.translation = position
        pole.matrix = matrix
    else:
        position = pole.matrix.translation.copy()
    ik = constraint(rig, key)
    tip = (rig.pose.bones[control_name(key)].matrix.translation.copy()
           if ik.influence > 0.5 and not ik.mute else middle.tail.copy())
    matrices = root.matrix.copy(), middle.matrix.copy()
    muted = ik.mute
    try:
        ik.mute = True
        context.view_layer.update()
        if preserve_pose:
            # Read/seed the unconstrained chain, not an already solved IK roll.
            for bone, matrix in zip((root, middle), matrices):
                bone.matrix = matrix
                context.view_layer.update()
        _seed_bend(context, root, middle, tip, position)
        axis = (middle.tail - root.head).normalized()
        bend = middle.head - root.head
        bend -= axis * bend.dot(axis)
        # IK_QJacobianSolver::ConstrainPoleVector uses X*cos(angle)+Z*sin(angle)
        # as its up vector. Calibrate against the seeded bend, not the pole's
        # world bearing; otherwise SL leg splay is retained during foot lifts.
        basis = root.matrix.to_3x3()
        ik.pole_angle = math.atan2(bend.dot(basis.col[2]), bend.dot(basis.col[0]))
    finally:
        ik.mute = muted
        context.view_layer.update()


def reposition_poles(context, rig):
    editable(rig)
    if not ready(rig):
        raise ValueError("Create Aurora IK controls first")
    if rig.data.pose_position != "POSE" or rig.mode == "EDIT":
        raise ValueError("Use Pose Position and leave Edit Mode to switch IK")
    context.view_layer.update()
    for key in LIMBS:
        _align_pole(context, rig, key)
    context.view_layer.update()


def correct_knees(context, rig):
    editable(rig)
    if not ready(rig):
        raise ValueError("Create Aurora IK controls first")
    if rig.data.pose_position != "POSE" or rig.mode == "EDIT":
        raise ValueError("Use Pose Position and leave Edit Mode to switch IK")
    keys = [key for key in ("FOOT_L", "FOOT_R") if constraint(rig, key).influence > 0.5 and not constraint(rig, key).mute]
    if not keys:
        raise ValueError("Enable foot IK before correcting knee bend")
    context.view_layer.update()
    for key in keys:
        _align_pole(context, rig, key, reposition=False, preserve_pose=False)


def setup(context, rig):
    editable(rig)
    if ready(rig):
        handles.apply(rig)
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
            pole_position = _pole(root.head, middle.head, tip, *_pole_settings(rig, key), follow_bend=False)
            size = max((middle.head - root.head).length * 0.3, 0.001)
            for pole in (False, True):
                name = control_name(key, pole)
                if name in rig.data.edit_bones:
                    continue
                bone = rig.data.edit_bones.new(name)
                parent = rig.data.edit_bones.get("auroraPose.Root")
                if parent and parent.get("aurora_control_kind") == "ROOT":
                    bone.parent = parent
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
        rig["aurora_ik_version"] = 3
        handles.apply(rig)
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
        _align_pole(context, rig, key)
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
    select_named_control(context, rig, control_name(key, pole), "IK")


def select_named_control(context, rig, name, part):
    if name not in rig.data.bones or not rig.data.bones[name].get("aurora_control"):
        raise ValueError("Create the pose control first")
    if context.object and context.object.mode != "OBJECT":
        bpy.ops.object.mode_set(mode="OBJECT")
    for obj in context.selected_objects:
        obj.select_set(False)
    rig.hide_set(False)
    rig.select_set(True)
    context.view_layer.objects.active = rig
    show(rig, part, True)
    for bone in rig.data.bones:
        _select(rig, bone, False)
    bone = rig.data.bones[name]
    _select(rig, bone, True)
    rig.data.bones.active = bone
    bpy.ops.object.mode_set(mode="POSE")
