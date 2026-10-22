"""Automatic heat weights on temporary objects; publish only complete results."""
import bpy

BODY = {"mPelvis", "mTorso", "mChest", "mNeck", "mHead"} | {
    "m" + name + side for name in ("Collar", "Shoulder", "Elbow", "Wrist", "Hip", "Knee", "Ankle", "Foot")
    for side in ("Left", "Right")
}


def _deforms(bone, scope):
    if not bone.use_deform:
        return False
    name = str(bone.get("aurora_joint", bone.name))
    return (name in BODY if scope == "BODY" else bone.use_deform and not bone.get("sl_collision_volume", False)
            and name not in {"mSpine1", "mSpine2", "mSpine3", "mSpine4"})


def _calculate(context, source, rig, scope):
    clone, temporary = source.copy(), rig.copy()
    clone.data, temporary.data = source.data.copy(), rig.data.copy()
    clone.parent, temporary.parent = None, None
    clone.constraints.clear()
    temporary.constraints.clear()
    clone.animation_data_clear()
    clone.matrix_world, temporary.matrix_world = source.matrix_world, rig.matrix_world
    temporary.animation_data_clear()
    temporary.data.pose_position = "REST"
    context.scene.collection.objects.link(clone)
    context.scene.collection.objects.link(temporary)
    for obj in (clone, temporary):
        obj.hide_viewport = False
        obj.hide_select = False
        obj.hide_set(False)
    try:
        for modifier in list(clone.modifiers):
            clone.modifiers.remove(modifier)
        clone.vertex_groups.clear()
        for bone in temporary.data.bones:
            bone.use_deform = _deforms(bone, scope)
        if not any(bone.use_deform for bone in temporary.data.bones):
            raise ValueError("No suitable deform bones in the chosen rig")
        for obj in context.selected_objects:
            obj.select_set(False)
        clone.select_set(True)
        temporary.select_set(True)
        context.view_layer.objects.active = temporary
        bpy.ops.object.parent_set(type="ARMATURE_AUTO", keep_transform=True)
        allowed = {bone.name for bone in temporary.data.bones if bone.use_deform}
        names = {group.index: group.name for group in clone.vertex_groups if group.name in allowed}
        result = []
        for vertex in clone.data.vertices:
            values = sorted(((names[g.group], g.weight) for g in vertex.groups
                             if g.group in names and g.weight > 0.000001), key=lambda pair: (-pair[1], pair[0]))[:4]
            total = sum(weight for _, weight in values)
            if total <= 0.000001:
                raise ValueError("Automatic weights failed; the original weights were kept. Check the mesh topology.")
            result.append([(name, weight / total) for name, weight in values])
        return result
    finally:
        for obj in (clone, temporary):
            data = obj.data
            bpy.data.objects.remove(obj, do_unlink=True)
            if data.users == 0:
                (bpy.data.meshes if isinstance(data, bpy.types.Mesh) else bpy.data.armatures).remove(data)


def rebind(context, objects, target=None, scope="BODY"):
    if not objects:
        raise ValueError("Select one or more meshes to recalculate their weights")
    selected, active = list(context.selected_objects), context.view_layer.objects.active
    mode = active.mode if active else "OBJECT"
    plans = []
    try:
        if mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        for source in objects:
            rig = target or source.find_armature()
            if rig is None:
                candidates = [obj for obj in selected if obj.type == "ARMATURE"]
                rig = candidates[0] if len(candidates) == 1 else None
            if rig is None:
                raise ValueError("Choose an avatar rig or select one armature with the meshes")
            if source.library or rig.library or len([m for m in source.modifiers if m.type == "ARMATURE"]) > 1:
                raise ValueError("Use editable objects with at most one Armature modifier")
            if not source.data.vertices:
                raise ValueError("The selected mesh has no vertices")
            plans.append((source, rig, _calculate(context, source, rig, scope)))
        # No source has changed if any heat solve above failed.
        for source, rig, values in plans:
            if source.data.users > 1 or source.data.library:
                source.data = source.data.copy()
            for group in list(source.vertex_groups):
                if group.name in rig.data.bones:
                    source.vertex_groups.remove(group)
            # Empty counterparts let Mirror modifiers swap left/right groups.
            groups = {bone.name: source.vertex_groups.new(name=bone.name)
                      for bone in rig.data.bones if _deforms(bone, scope)}
            for index, influences in enumerate(values):
                for name, weight in influences:
                    group = groups.get(name)
                    if group is None:
                        group = groups[name] = source.vertex_groups.new(name=name)
                    group.add([index], weight, "REPLACE")
            modifier = next((m for m in source.modifiers if m.type == "ARMATURE"), None)
            if modifier is None:
                modifier = source.modifiers.new("Aurora Avatar Rig", "ARMATURE")
            modifier.object = rig
            if source.parent is not rig:
                world = source.matrix_world.copy()
                source.parent = rig
                source.matrix_parent_inverse = rig.matrix_world.inverted()
                source.matrix_world = world
            source.data.update()
    finally:
        for obj in context.selected_objects:
            obj.select_set(False)
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        if mode != "OBJECT" and active:
            bpy.ops.object.mode_set(mode=mode)
        context.view_layer.update()
