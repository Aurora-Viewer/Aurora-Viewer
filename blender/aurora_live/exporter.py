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
from mathutils.kdtree import KDTree
from .materials import Capture

MAX_CORNERS = 600_000
VERTEX_DTYPE = np.dtype([("position", "<f4", 3), ("normal", "<f4", 3), ("uv", "<f4", 2),
                         ("joints", "u1", 4), ("weights", "<f4", 4)])


def influences(groups, group_names, palette):
    values = []
    for group in groups:
        name = group_names.get(group.group)
        if name is not None and not math.isfinite(group.weight):
            raise ValueError("Avatar bone weights must be finite")
        if name is not None and group.weight > 0.0:
            if name not in palette:
                raise ValueError("Unsupported weighted bone: " + name)
            values.append((palette[name], float(group.weight)))
    values.sort(key=lambda pair: (-pair[1], pair[0]))
    values = values[:4]
    total = sum(weight for _, weight in values)
    if not math.isfinite(total):
        raise ValueError("Avatar bone weights must be finite")
    if total <= 0.000001:
        return [0] * 4, [0.0] * 4
    joints = [joint for joint, _ in values]
    weights = [weight / total for _, weight in values]
    return joints + [0] * (4 - len(joints)), weights + [0.0] * (4 - len(weights))


def _complete_weights(weights, positions):
    """Complete the exported copy, using mesh import's normalized-distance rule."""
    missing = [index for index, value in weights.items() if not any(value[1])]
    if not missing:
        return
    weighted = [index for index, value in weights.items() if any(value[1])]
    if not weighted:
        for index in missing:
            weights[index] = ([0] * 4, [1.0, 0.0, 0.0, 0.0])
        return
    points = np.asarray(positions)
    referenced = points[list(weights)]
    if not np.isfinite(referenced).all():
        raise ValueError("Mesh positions must be finite")
    points = points / np.maximum(np.ptp(referenced, axis=0), 1e-6)
    tree = KDTree(len(weighted))
    for index in weighted:
        tree.insert(Vector(points[index]), index)
    tree.balance()
    for index in missing:
        _point, nearest, _distance = tree.find(Vector(points[index]))
        weights[index] = weights[nearest]


def _weight_arrays(mesh, referenced, group_names, group_bones):
    """Read RNA weights once; rank and normalize in NumPy after palette creation."""
    vertices, groups, values, used_groups = [], [], [], set()
    for index in referenced:
        for group in mesh.vertices[int(index)].groups:
            identity = group.group
            if identity not in group_names:
                continue
            value = float(group.weight)
            if not math.isfinite(value):
                raise ValueError("Avatar bone weights must be finite")
            if value > 0:
                vertices.append(index)
                groups.append(identity)
                values.append(value)
                used_groups.add(identity)
    return (np.asarray(vertices, dtype=np.intp), np.asarray(groups, dtype=np.intp), np.asarray(values, dtype=np.float64),
            {group_names[group] for group in used_groups}, {group_bones[group] for group in used_groups})


def _dense_influences(count, assignments, group_names, palette):
    joints = np.zeros((count, 4), dtype=np.uint8)
    weights = np.zeros((count, 4), dtype=np.float64)
    vertices, groups, values = assignments[:3]
    if len(vertices):
        mapping = np.full(max(group_names) + 1, -1, dtype=np.int16)
        for group, name in group_names.items():
            mapping[group] = palette.get(name, -1)
        identities = mapping[groups]
        order = np.lexsort((identities, -values, vertices))
        sorted_vertices = vertices[order]
        offsets = np.arange(len(order))
        starts = np.r_[True, sorted_vertices[1:] != sorted_vertices[:-1]]
        rank = offsets - np.maximum.accumulate(np.where(starts, offsets, 0))
        selected = rank < 4
        rows, columns = sorted_vertices[selected], rank[selected]
        joints[rows, columns] = identities[order[selected]]
        weights[rows, columns] = values[order[selected]]
        # Preserve the strongest-four summation order of influences().
        total = ((weights[:, 0] + weights[:, 1]) + weights[:, 2]) + weights[:, 3]
        if not np.isfinite(total).all():
            raise ValueError("Avatar bone weights must be finite")
        weights /= np.where(total > 1e-6, total, 1)[:, None]
        weights[total <= 1e-6] = 0
        joints[total <= 1e-6] = 0
    return joints, weights


def _complete_dense_weights(joints, weights, positions, referenced):
    assigned = np.any(weights[referenced], axis=1)
    missing = referenced[~assigned]
    if not len(missing):
        return
    weighted = referenced[assigned]
    if not len(weighted):
        weights[missing, 0] = 1
        return
    if not np.isfinite(positions[referenced]).all():
        raise ValueError("Mesh positions must be finite")
    points = positions / np.maximum(np.ptp(positions[referenced], axis=0), 1e-6)
    tree = KDTree(len(weighted))
    for index in weighted:
        tree.insert(Vector(points[index]), int(index))
    tree.balance()
    for index in missing:
        _point, nearest, _distance = tree.find(Vector(points[index]))
        joints[index], weights[index] = joints[nearest], weights[nearest]


def _column_major(matrix):
    return [float(matrix[row][column]) for column in range(4) for row in range(4)]


def bind_rotation(armature, bone, bone_transform):
    """Rotation of the joint in Avastar's bind pose, or identity.

    Avastar rigs can bind meshes in a rest pose that is not SL's T-pose (body
    dev kits lower the arms into an A-pose). Each rotated bone keeps that pose
    in `bind_mat` (SL axes, column-major); its DAE export un-rotates vertices
    by it, so LiveMesh must too (#1455). A bone without one keeps its nearest
    ancestor's. Bind data whose origin is no longer the bone head (rest pose
    edited since, other coordinate convention) is ignored.
    """
    if not armature.get("has_bind_data"):
        return Matrix.Identity(4)
    while bone is not None and "bind_mat" not in bone.keys():
        bone = bone.parent
    if bone is None:
        return Matrix.Identity(4)
    values = [float(value) for value in bone["bind_mat"]]
    if len(values) != 16 or not all(math.isfinite(value) for value in values):
        return Matrix.Identity(4)
    bind = Matrix([values[i:i + 4] for i in range(0, 16, 4)]).transposed()
    if (bind.translation - bone_transform @ bone.head_local).length > 0.001:
        return Matrix.Identity(4)
    return bind.to_quaternion().to_matrix().to_4x4()


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


def _mesh(source, supported, forward, rest_axes, capture, include_slots=False, packed=False):
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
    # Base meshes already contain the rest coordinates needed here. Avoid
    # linking/removing a clone, which reevaluates an entire complex scene.
    direct = (packed and source.mode in {"OBJECT", "EDIT"} and source.data.shape_keys is None
              and all(modifier.type == "ARMATURE" or not modifier.show_viewport for modifier in source.modifiers))
    if direct:
        # Some older edit-mode APIs expose UVs only on an evaluated mesh.
        base_uv = source.data.uv_layers.active
        direct = base_uv is None or len(base_uv.data) == len(source.data.loops)
    clone = None
    if not direct:
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
        if direct:
            captured = source.data
        else:
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
        assignments = None
        if packed and armature:
            assignments = _weight_arrays(captured, referenced, group_names, group_bones)
            used, weighted_bones = assignments[3:]
        elif not packed:
            for index in referenced:
                vertex = captured.vertices[int(index)]
                for group in vertex.groups:
                    if group.weight > 0.0 and group.group in group_names:
                        used.add(group_names[group.group])
                        weighted_bones.add(group_bones[group.group])
        if armature and not used:
            # A new, wholly unweighted mesh stays attached while weights are
            # authored. Prefer the pelvis, then the shallowest supported bone.
            candidates = [name for name, joint in bone_names.items() if joint in supported]
            if candidates:
                source_name = min(candidates, key=lambda name: (
                    bone_names[name] != "mPelvis", len(armature.data.bones[name].parent_recursive), name))
                used.add(bone_names[source_name])
                weighted_bones.add(source_name)
        names = sorted(used)
        if armature and (not names or len(names) > 110):
            raise ValueError(source.name + ": expected 1 to 110 weighted avatar joints")
        unknown = used - supported
        if unknown:
            raise ValueError("Unsupported weighted bones: " + ", ".join(sorted(unknown)))
        palette = {name: index for index, name in enumerate(names)}
        if packed:
            dense_joints, dense_weights = (_dense_influences(len(captured.vertices), assignments, group_names, palette)
                                          if armature else (None, None))
        else:
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
                rotation = bind_rotation(armature, source_bone, bone_transform)
                inverse_bind.append(_column_major(axes @ rotation.transposed() @ Matrix.Translation(-heads[name])))
        positions = np.empty(len(captured.vertices) * 3, dtype=np.float32)
        captured.vertices.foreach_get("co", positions)
        positions = np.einsum("ij,kj->ki", np.array(transform.to_3x3()), positions.reshape((-1, 3))) + np.array(transform.translation)
        if packed:
            if armature:
                _complete_dense_weights(dense_joints, dense_weights, positions, referenced)
        else:
            positions = positions.tolist()
            if armature:
                _complete_weights(weights, positions)
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
        if not packed:
            normals = normals.tolist()
        uv_layer = captured.uv_layers.active
        uv = np.zeros((len(captured.loops), 2), dtype=np.float32)
        if uv_layer:
            uv_layer.data.foreach_get("uv", uv.reshape(-1))
        if not packed:
            uv = uv.tolist()
        surfaces = []
        for slot in np.unique(materials):
            loops, indices = np.unique(triangle_loops[materials == slot], return_inverse=True)
            color, material = capture.material(source, int(slot))
            if packed:
                # The detached bytes contain no bpy references. Avoid creating
                # hundreds of thousands of dicts, then packing them again later.
                records = np.zeros(len(loops), dtype=VERTEX_DTYPE)
                vertex_indices = loop_vertices[loops]
                records["position"] = positions[vertex_indices]
                records["normal"] = normals[loops]
                records["uv"] = uv[loops]
                if armature:
                    records["joints"] = dense_joints[vertex_indices]
                    records["weights"] = dense_weights[vertex_indices]
                surfaces.append({"color": color, "material": material,
                                 "_buffers": (records.tobytes(), indices.astype("<u4").reshape(-1).tobytes())})
            else:
                vertices = []
                for index in loops.tolist():
                    vertex_index = int(loop_vertices[index])
                    joints, vertex_weights = weights[vertex_index]
                    vertices.append({"position": positions[vertex_index], "normal": normals[index],
                        "uv": uv[index], "joints": joints, "weights": vertex_weights})
                surfaces.append({"color": color, "material": material, "vertices": vertices, "indices": indices.reshape(-1).tolist()})
        if not surfaces or len(surfaces) > 32:
            raise ValueError(source.name + ": expected 1 to 32 nonempty material surfaces")
        result = {"name": source.name, "joints": names, "inverse_bind": inverse_bind, "faces": surfaces}
        if include_slots or packed:
            result["_slots"] = [int(slot) for slot in np.unique(materials)]
        return result
    finally:
        if evaluated is not None and captured is not None:
            evaluated.to_mesh_clear()
        if clone is not None:
            data = clone.data
            bpy.data.objects.remove(clone, do_unlink=True)
            if data.users == 0:
                bpy.data.meshes.remove(data)
            # Flush removal notifications while the caller still suppresses
            # our capture changes; otherwise they dirty the source next tick.
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


def material_snapshot(objects, previous, source_ids, revision, errors, warnings=None):
    """Reuse detached geometry when only materials or texture pixels changed."""
    if not previous or len(previous["meshes"]) != len(objects):
        return None
    by_id = {mesh.get("source_id"): mesh for mesh in previous["meshes"]}
    if any(source_ids.get(obj.as_pointer()) not in by_id for obj in objects):
        return None
    if any(len(mesh.get("_slots", [])) != len(mesh["faces"]) for mesh in by_id.values()):
        return None
    capture, meshes = Capture(lenient=warnings is not None), []
    for source in objects:
        before = len(capture.images), dict(capture.ids), capture.bytes
        mesh = by_id[source_ids[source.as_pointer()]]
        faces = []
        capture.warnings = []
        try:
            for slot, face in zip(mesh["_slots"], mesh["faces"]):
                color, material = capture.material(source, slot)
                faces.append(dict(face, color=color, material=material))
        except Exception as error:
            del capture.images[before[0]:]
            capture.ids, capture.bytes = before[1:]
            errors[source.as_pointer()] = str(error) or type(error).__name__
            continue
        if capture.warnings:
            warnings[source.as_pointer()] = capture.warnings[0]
        meshes.append(dict(mesh, faces=faces))
    _valid_links(meshes)
    return dict(previous, revision=revision, meshes=meshes, images=capture.images)


def _valid_links(meshes):
    identities = {mesh.get("source_id") for mesh in meshes}
    for mesh in meshes:
        if mesh.get("link", {}).get("parent") not in identities:
            mesh.pop("link", None)


def snapshot(objects, supported, revision, forward="AUTO", rest_axes=None, source_ids=None, link_roots=None, errors=None, packed=False,
             warnings=None):
    """Capture strictly for imports, or isolate live source failures in errors.

    With warnings, a material that cannot be captured falls back to its plain
    colour and the source keeps syncing; the reason is kept in warnings.
    """
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
    capture = Capture(lenient=warnings is not None)
    corners = 0
    for source in objects:
        try:
            exists = source.name in bpy.data.objects
        except ReferenceError:
            exists = False
        if exists and source.type == "MESH":
            # A failed source may already have stored images. Roll back its
            # budget and IDs so it cannot poison later, otherwise valid sources.
            image_count, image_ids, image_bytes = len(capture.images), dict(capture.ids), capture.bytes
            capture.warnings = []
            try:
                mesh = (_mesh(source, supported, forward, rest_axes or {}, capture, packed=True) if packed else
                        _mesh(source, supported, forward, rest_axes or {}, capture))
                count = sum(len(face["_buffers"][0]) // VERTEX_DTYPE.itemsize if "_buffers" in face else len(face["vertices"])
                            for face in mesh["faces"])
                if len(meshes) >= 32:
                    raise ValueError("Select at most 32 mesh objects")
                if corners + count > MAX_CORNERS:
                    raise ValueError("Snapshot exceeds 600,000 mesh corners")
                if source_ids and source.as_pointer() in source_ids:
                    mesh["source_id"] = source_ids[source.as_pointer()]
                    root = roots.get(source.as_pointer())
                    if root is not None and root != source:
                        if root.as_pointer() in source_ids:
                            basis = coordinate_basis(None, "NATIVE" if forward == "AUTO" else forward)
                            offset = basis.to_3x3() @ (source.matrix_world.translation - root.matrix_world.translation)
                            mesh["link"] = {"parent": source_ids[root.as_pointer()],
                                            "offset": list(offset * bpy.context.scene.unit_settings.scale_length)}
            except Exception as error:
                if errors is None:
                    raise
                del capture.images[image_count:]
                capture.ids, capture.bytes = image_ids, image_bytes
                errors[source.as_pointer()] = str(error) or type(error).__name__
                continue
            if capture.warnings:
                warnings[source.as_pointer()] = capture.warnings[0]
            meshes.append(mesh)
            corners += count
    # Failed roots must not leave dangling links that reject the whole frame.
    _valid_links(meshes)
    return {"protocol": 1, "revision": revision, "meshes": meshes, "images": capture.images}
