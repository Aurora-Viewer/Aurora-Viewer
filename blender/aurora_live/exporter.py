"""Rest-mesh capture on Blender's main thread; source objects are untouched.

Temporary copies evaluate shape keys and geometry modifiers without armature
deformation. SL uses joint coordinate axes, not Blender bone roll: default
inverse binds translate the source rest head to the joint origin. Custom
bone mapping uses the bone's optional `aurora_joint` string property.
"""

import math
import bpy
import numpy as np
from mathutils import Matrix, Vector
from .materials import Capture

MAX_CORNERS = 600_000


def influences(groups, group_names, palette):
    values = []
    for group in groups:
        name = group_names.get(group.group)
        if name is not None and group.weight > 0.0:
            if name not in palette:
                raise ValueError("Unsupported weighted bone: " + name)
            values.append((palette[name], float(group.weight)))
    values.sort(key=lambda pair: (-pair[1], pair[0]))
    values = values[:4]
    total = sum(weight for _, weight in values)
    if not math.isfinite(total) or total <= 0.000001:
        raise ValueError("Every vertex must have an avatar bone weight")
    joints = [joint for joint, _ in values]
    weights = [weight / total for _, weight in values]
    return joints + [0] * (4 - len(joints)), weights + [0.0] * (4 - len(weights))


def _column_major(matrix):
    return [float(matrix[row][column]) for column in range(4) for row in range(4)]


def armature_for(source):
    # A prop parented to a bone is rigid. find_armature() alone also returns
    # that parent even though no armature modifier deforms the mesh.
    return source.find_armature() if any(mod.type == "ARMATURE" and mod.object for mod in source.modifiers) else None


def coordinate_basis(armature, forward="AUTO"):
    """Map rest-data axes to SL +X; object placement is removed separately.

    Avastar faces Blender -Y, while older Aurora/SL rigs face +X. Rest joint
    pairs identify the left axis without relying on bone roll, current pose or
    vendor properties. Capture remains independent for each armature.
    """
    if forward == "AUTO":
        bones = {str(bone.get("aurora_joint", bone.name)): bone for bone in armature.data.bones
                 if not bone.get("aurora_control", False)}
        for stem in ("mShoulder", "mCollar", "mHip", "mEye", "mWrist"):
            left, right = (bones.get(stem + side) for side in ("Left", "Right"))
            if left is None or right is None:
                continue
            direction = left.head_local - right.head_local
            if direction.x * direction.x + direction.y * direction.y < 1e-12:
                continue
            angle = math.pi / 2 - math.atan2(direction.y, direction.x)
            return Matrix.Identity(4) if abs(math.remainder(angle, math.tau)) < 1e-6 else Matrix.Rotation(angle, 4, "Z")
        # Partial older rigs do not always contain a symmetric joint pair.
        forward = armature.get("aurora_forward", "NATIVE")
    return Matrix.Rotation(math.pi / 2, 4, "Z") if forward == "NEGATIVE_Y" else Matrix.Identity(4)


def _mesh(source, supported, forward, rest_axes, capture, include_slots=False):
    source.update_from_editmode()
    armature = armature_for(source)
    # Ignore the armature's scene placement but retain its authored scale.
    root = (Matrix.Translation(armature.matrix_world.translation) @ armature.matrix_world.to_quaternion().to_matrix().to_4x4()
            if armature else Matrix.Translation(source.matrix_world.translation))
    unit = bpy.context.scene.unit_settings.scale_length
    basis = coordinate_basis(armature, forward) if armature else coordinate_basis(None, "NATIVE" if forward == "AUTO" else forward)
    transform = basis @ Matrix.Scale(unit, 4) @ root.inverted() @ source.matrix_world
    bone_transform = basis @ Matrix.Scale(unit, 4) @ root.inverted() @ armature.matrix_world if armature else None
    normal_transform = transform.to_3x3().inverted().transposed()
    bone_names = {bone.name: str(bone.get("aurora_joint", bone.name)) for bone in (armature.data.bones if armature else [])
                  if bone.use_deform and not bone.get("aurora_control", False)}
    group_names = {group.index: bone_names[group.name] for group in source.vertex_groups if group.name in bone_names}
    group_bones = {group.index: group.name for group in source.vertex_groups if group.name in bone_names}
    clone = source.copy()
    clone.data = source.data.copy()
    clone.name = "_AuroraLiveCapture"
    clone.parent = None
    clone.matrix_world = source.matrix_world
    bpy.context.scene.collection.objects.link(clone)
    clone.hide_viewport = False
    clone.hide_set(False)
    captured = None
    evaluated = None
    try:
        for modifier in list(clone.modifiers):
            if modifier.type == "ARMATURE":
                clone.modifiers.remove(modifier)
        depsgraph = bpy.context.evaluated_depsgraph_get()
        depsgraph.update()
        evaluated = clone.evaluated_get(depsgraph)
        captured = evaluated.to_mesh(preserve_all_data_layers=True, depsgraph=depsgraph)
        if len(captured.loops) > MAX_CORNERS:
            raise ValueError(source.name + ": more than 600,000 mesh corners")
        captured.calc_loop_triangles()
        if not captured.loop_triangles:
            raise ValueError(source.name + ": no triangle surfaces")
        # Bulk Blender reads and array transforms keep the timer responsive.
        # NumPy ships with Blender; no package installation is required.
        loop_vertices = np.empty(len(captured.loops), dtype=np.int32)
        captured.loops.foreach_get("vertex_index", loop_vertices)
        triangle_loops = np.empty(len(captured.loop_triangles) * 3, dtype=np.int32)
        captured.loop_triangles.foreach_get("loops", triangle_loops)
        triangle_loops = triangle_loops.reshape((-1, 3))
        if transform.determinant() < 0:
            triangle_loops = triangle_loops[:, ::-1]
        materials = np.empty(len(captured.loop_triangles), dtype=np.int32)
        captured.loop_triangles.foreach_get("material_index", materials)
        referenced = np.unique(loop_vertices[triangle_loops])
        used = set()
        weighted_bones = set()
        for index in referenced:
            vertex = captured.vertices[int(index)]
            for group in vertex.groups:
                if group.weight > 0.0 and group.group in group_names:
                    used.add(group_names[group.group])
                    weighted_bones.add(group_bones[group.group])
        names = sorted(used)
        if armature and (not names or len(names) > 110):
            raise ValueError(source.name + ": expected 1 to 110 weighted avatar joints")
        unknown = used - supported
        if unknown:
            raise ValueError("Unsupported weighted bones: " + ", ".join(sorted(unknown)))
        palette = {name: index for index, name in enumerate(names)}
        weights = {int(index): (influences(captured.vertices[int(index)].groups, group_names, palette)
                               if armature else ([0] * 4, [0.0] * 4)) for index in referenced}
        heads = {}
        for source_name in sorted(weighted_bones):
            name = bone_names[source_name]
            if name in heads:
                raise ValueError("Multiple weighted bones map to the same avatar joint: " + name)
            heads[name] = bone_transform @ armature.data.bones[source_name].head_local
        inverse_bind = []
        for name in names:
            values = rest_axes.get(name)
            axes = Matrix([values[i:i+4] for i in range(0,16,4)]).transposed() if values else Matrix.Identity(4)
            source_bone = next(armature.data.bones[source_name] for source_name in weighted_bones if bone_names[source_name] == name)
            if armature.get("aurora_shape_id"):
                values = list(source_bone.get("aurora_shape_bind", ()))
                reference = list(source_bone.get("aurora_shape_head", ()))
                if len(values) != 16 or len(reference) != 3 or not all(math.isfinite(value) for value in values + reference):
                    raise ValueError("Invalid transferred shape bind: " + name)
                shaped = Matrix([values[i:i+4] for i in range(0, 16, 4)]).transposed()
                if abs(shaped.determinant()) < 1e-8:
                    raise ValueError("Invalid transferred shape bind: " + name)
                # Cancel the transferred rest shape, including its scale, before
                # Aurora applies the wearer's current shape and animation.
                inverse_bind.append(_column_major(shaped @ Matrix.Translation(Vector(reference) - heads[name])))
            else:
                inverse_bind.append(_column_major(axes @ Matrix.Translation(-heads[name])))
        positions = np.empty(len(captured.vertices) * 3, dtype=np.float32)
        captured.vertices.foreach_get("co", positions)
        positions = np.einsum("ij,kj->ki", np.array(transform.to_3x3()), positions.reshape((-1, 3))) + np.array(transform.translation)
        positions = positions.tolist()
        normals = np.empty(len(captured.loops) * 3, dtype=np.float32)
        if hasattr(captured, "calc_normals_split"):
            # Before 4.1 even a present corner_normals collection can be empty
            # until explicitly calculated (notably in Blender 3.6).
            captured.calc_normals_split()
            captured.loops.foreach_get("normal", normals)
        else:
            captured.corner_normals.foreach_get("vector", normals)
        normals = np.einsum("ij,kj->ki", np.array(normal_transform), normals.reshape((-1, 3)))
        normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-20)
        normals = normals.tolist()
        uv_layer = captured.uv_layers.active
        uv = np.zeros((len(captured.loops), 2), dtype=np.float32)
        if uv_layer:
            uv_layer.data.foreach_get("uv", uv.reshape(-1))
        uv = uv.tolist()
        surfaces = []
        for slot in np.unique(materials):
            loops, indices = np.unique(triangle_loops[materials == slot], return_inverse=True)
            vertices = []
            for index in loops.tolist():
                vertex_index = int(loop_vertices[index])
                joints, vertex_weights = weights[vertex_index]
                vertices.append({"position": positions[vertex_index], "normal": normals[index],
                    "uv": uv[index], "joints": joints, "weights": vertex_weights})
            color, material = capture.material(source, int(slot))
            surfaces.append({"color": color, "material": material, "vertices": vertices, "indices": indices.reshape(-1).tolist()})
        if not surfaces or len(surfaces) > 32:
            raise ValueError(source.name + ": expected 1 to 32 nonempty material surfaces")
        result = {"name": source.name, "joints": names, "inverse_bind": inverse_bind, "faces": surfaces}
        if include_slots:
            result["_slots"] = [int(slot) for slot in np.unique(materials)]
        return result
    finally:
        if evaluated is not None and captured is not None:
            evaluated.to_mesh_clear()
        data = clone.data
        bpy.data.objects.remove(clone, do_unlink=True)
        if data.users == 0:
            bpy.data.meshes.remove(data)
        # Flush removal notifications while the caller still suppresses our
        # own capture changes; otherwise they dirty the source next tick.
        bpy.context.view_layer.update()


def static_roots(objects, overrides=None):
    """Resolve explicit LiveMesh groups and inherited Blender parents to one root."""
    static = {obj.as_pointer(): obj for obj in objects if armature_for(obj) is None}
    parents = {}
    for key, obj in static.items():
        if overrides is not None and key in overrides:
            parent = overrides[key]
        else:
            parent = obj.parent
            while parent is not None and parent.as_pointer() not in static:
                parent = parent.parent
        parents[key] = parent.as_pointer() if parent is not None and parent.as_pointer() in static else None
    roots = {}
    for key in static:
        chain, current = set(), key
        while parents[current] is not None:
            if current in chain:
                raise ValueError("Invalid LiveMesh grouping")
            chain.add(current)
            current = parents[current]
        roots[key] = static[current]
    return roots


def snapshot(objects, supported, revision, forward="AUTO", rest_axes=None, source_ids=None, link_roots=None):
    valid = []
    for source in objects:
        try:
            if source.name in bpy.data.objects and source.type == "MESH":
                valid.append(source)
        except ReferenceError:
            pass
    objects = valid
    roots = static_roots(objects, link_roots)
    meshes = []
    capture = Capture()
    for source in objects:
        try:
            exists = source.name in bpy.data.objects
        except ReferenceError:
            exists = False
        if exists and source.type == "MESH":
            mesh = _mesh(source, supported, forward, rest_axes or {}, capture)
            if source_ids and source.as_pointer() in source_ids:
                mesh["source_id"] = source_ids[source.as_pointer()]
                root = roots.get(source.as_pointer())
                if root is not None and root != source:
                    if root.as_pointer() in source_ids:
                        basis = coordinate_basis(None, "NATIVE" if forward == "AUTO" else forward)
                        offset = basis.to_3x3() @ (source.matrix_world.translation - root.matrix_world.translation)
                        mesh["link"] = {"parent": source_ids[root.as_pointer()],
                                        "offset": list(offset * bpy.context.scene.unit_settings.scale_length)}
            meshes.append(mesh)
    if len(meshes) > 32:
        raise ValueError("Select at most 32 mesh objects")
    corners = sum(len(face["vertices"]) for mesh in meshes for face in mesh["faces"])
    if corners > MAX_CORNERS:
        raise ValueError("Snapshot exceeds 600,000 mesh corners")
    return {"protocol": 1, "revision": revision, "meshes": meshes, "images": capture.images}
