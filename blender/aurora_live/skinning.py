"""Transfer/clean SL weights on temporary donor geometry, then publish complete plans."""
import bpy
from mathutils.bvhtree import BVHTree
from mathutils.kdtree import KDTree


def _allowed(rig):
    return {bone.name for bone in rig.data.bones if bone.use_deform and not bone.get("aurora_control", False)}


def _normalized(values):
    values = sorted(((name, weight) for name, weight in values.items() if weight > 0.000001),
                    key=lambda item: (-item[1], item[0]))[:4]
    total = sum(weight for _, weight in values)
    if total <= 0.000001:
        raise ValueError("Some vertices have no transferable deform weights; original weights were kept")
    return [(name, weight / total) for name, weight in values]


def _validate(source, rig):
    if source.library or rig.library or len([m for m in source.modifiers if m.type == "ARMATURE"]) > 1:
        raise ValueError("Use editable objects with at most one Armature modifier")
    if not source.data.vertices:
        raise ValueError("The selected mesh has no vertices")
    if any(group.lock_weight for group in source.vertex_groups if group.name in rig.data.bones):
        raise ValueError("Unlock the target's bone vertex groups before replacing weights")


def _publish(source, rig, values):
    if source.data.users > 1 or source.data.library:
        source.data = source.data.copy()
    for group in list(source.vertex_groups):
        if group.name in rig.data.bones:
            source.vertex_groups.remove(group)
    groups = {name: source.vertex_groups.new(name=name) for name in sorted(_allowed(rig))}
    for index, influences in enumerate(values):
        for name, weight in influences:
            groups[name].add([index], weight, "REPLACE")
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


def _donor(context, source, geometry_modifiers=True):
    clone = source.copy()
    clone.data = source.data.copy()
    clone.parent = None
    clone.constraints.clear()
    clone.animation_data_clear()
    clone.matrix_world = source.matrix_world
    context.scene.collection.objects.link(clone)
    clone.hide_viewport = False
    clone.hide_set(False)
    evaluated = None
    try:
        for modifier in list(clone.modifiers):
            if not geometry_modifiers or modifier.type == "ARMATURE":
                clone.modifiers.remove(modifier)
        context.view_layer.update()
        depsgraph = context.evaluated_depsgraph_get()
        evaluated = clone.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=depsgraph)
        mesh.calc_loop_triangles()
        vertices = [clone.matrix_world @ vertex.co for vertex in mesh.vertices]
        triangles = [tuple(triangle.vertices) for triangle in mesh.loop_triangles]
        names = {group.index: group.name for group in clone.vertex_groups}
        weights = [{names[g.group]: g.weight for g in vertex.groups if g.group in names} for vertex in mesh.vertices]
        return vertices, triangles, weights
    finally:
        if evaluated:
            evaluated.to_mesh_clear()
        data = clone.data
        bpy.data.objects.remove(clone, do_unlink=True)
        if not data.users:
            bpy.data.meshes.remove(data)


def _barycentric(point, a, b, c):
    u, v, w = b - a, c - a, point - a
    aa, ab, bb, wa, wb = u.dot(u), u.dot(v), v.dot(v), w.dot(u), w.dot(v)
    determinant = aa * bb - ab * ab
    if abs(determinant) < 1e-20:
        return (1, 0, 0)
    y, z = (bb * wa - ab * wb) / determinant, (aa * wb - ab * wa) / determinant
    return tuple(max(0, value) for value in (1 - y - z, y, z))


def apply(context, objects, rig=None, donor=None, method="SURFACE", max_distance=0, clean=False):
    targets = [obj for obj in objects if obj is not donor]
    if not targets:
        raise ValueError("Select target meshes other than the weight source")
    if not clean and (donor is None or donor.type != "MESH"):
        raise ValueError("Choose a mesh as the weight source")
    active = context.view_layer.objects.active
    selected = list(context.selected_objects)
    mode = active.mode if active else "OBJECT"
    plans = []
    try:
        if mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        if not clean:
            vertices, triangles, weights = _donor(context, donor)
            if not vertices or (method == "SURFACE" and not triangles):
                raise ValueError("The weight source needs vertices and, for surface transfer, faces")
            tree = BVHTree.FromPolygons(vertices, triangles, all_triangles=True) if method == "SURFACE" else KDTree(len(vertices))
            if method != "SURFACE":
                for index, position in enumerate(vertices):
                    tree.insert(position, index)
                tree.balance()
        for source in targets:
            target = rig or source.find_armature() or (donor.find_armature() if donor else None)
            if target is None:
                raise ValueError("Choose an avatar rig or select one armature with the meshes")
            _validate(source, target)
            allowed = _allowed(target)
            if clean:
                names = {group.index: group.name for group in source.vertex_groups if group.name in allowed}
                values = [_normalized({names[g.group]: g.weight for g in vertex.groups if g.group in names})
                          for vertex in source.data.vertices]
            else:
                canonical = {str(bone.get("aurora_joint", bone.name)): bone.name for bone in target.data.bones if bone.name in allowed}
                donor_rig = donor.find_armature()
                mapping = {group.name: canonical.get(str(donor_rig.data.bones[group.name].get("aurora_joint", group.name))
                           if donor_rig and group.name in donor_rig.data.bones else group.name) for group in donor.vertex_groups}
                values = []
                # Match the current fitted shape while retaining original vertex
                # indices. Topology modifiers and armature posing are excluded.
                positions = (_donor(context, source, geometry_modifiers=False)[0] if source.data.shape_keys else
                             [source.matrix_world @ vertex.co for vertex in source.data.vertices])
                for point in positions:
                    if method == "SURFACE":
                        nearest, _normal, index, distance = tree.find_nearest(point)
                        if index is None:
                            raise ValueError("No nearby source surface; original weights were kept")
                        indices = triangles[index]
                        factors = _barycentric(nearest, *(vertices[i] for i in indices))
                    else:
                        _nearest, index, distance = tree.find(point)
                        indices, factors = (index,), (1,)
                    if max_distance > 0 and distance > max_distance:
                        raise ValueError("A target vertex exceeds the transfer distance; original weights were kept")
                    combined = {}
                    for index, factor in zip(indices, factors):
                        for name, weight in weights[index].items():
                            mapped = mapping.get(name)
                            if mapped:
                                combined[mapped] = combined.get(mapped, 0) + weight * factor
                    values.append(_normalized(combined))
            plans.append((source, target, values))
        # Validate every target and every vertex before publishing any weights.
        for source, target, values in plans:
            _publish(source, target, values)
        return targets
    finally:
        for obj in context.selected_objects:
            obj.select_set(False)
        for obj in selected:
            obj.select_set(True)
        context.view_layer.objects.active = active
        if active and mode != "OBJECT":
            bpy.ops.object.mode_set(mode=mode)
        context.view_layer.update()
