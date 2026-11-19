"""Rest-mesh capture on Maya's main thread; nothing in the scene is changed.

A skinned mesh is read from its skinCluster's input geometry: the mesh after
blend shapes and other deformers that come before the skin, before the joints
move it, as the Blender plug-in reads a mesh without its armature modifier.
Its positions are taken to the bind pose with the skin's geomMatrix (the
Collada bind shape matrix) and each joint's bindPreMatrix gives its inverse
bind, as a Collada export from Maya writes them (LLDAELoader reads the same
data at upload, indra/llprimitive/lldaeloader.cpp). A mesh without a skin keeps
its rotation and scale and drops its world position, like the Blender plug-in.
"""

import maya.api.OpenMaya as om
import maya.api.OpenMayaAnim as oma
from maya import cmds

from . import convert, materials


def _dag(name):
    selection = om.MSelectionList()
    selection.add(name)
    return selection.getDagPath(0)


def shape_of(transform):
    """The visible mesh shape of a transform, or None."""
    shapes = cmds.listRelatives(transform, shapes=True, noIntermediate=True, fullPath=True, type="mesh") or []
    return shapes[0] if shapes else None


def skin_of(shape):
    """The skinCluster that deforms this shape, or None."""
    for node in cmds.listHistory(shape, pruneDagObjects=True, interestLevel=1) or []:
        if cmds.nodeType(node) == "skinCluster":
            return node
    return None


def _values(matrix):
    return [matrix[index] for index in range(16)]


def _maya_matrix(plug):
    return convert.from_maya(_values(om.MFnMatrixData(plug.asMObject()).matrix()))


def _node_plug(node, name):
    selection = om.MSelectionList()
    selection.add(node)
    return om.MFnDependencyNode(selection.getDependNode(0)).findPlug(name, False)


def _joint_override(path):
    node = om.MFnDependencyNode(path.node())
    if node.hasAttribute("auroraJoint"):
        try:
            return node.findPlug("auroraJoint", False).asString()
        except RuntimeError:
            return None
    return None


def _skin(shape_path, skin, supported):
    """Joint names, their bind-pre matrices, the bind shape matrix, the input
    mesh and each vertex's influences by SL joint name."""
    selection = om.MSelectionList()
    selection.add(skin)
    skin_object = selection.getDependNode(0)
    fn = oma.MFnSkinCluster(skin_object)
    index = fn.indexForOutputShape(shape_path.node())
    geometry = _node_plug(skin, "input").elementByLogicalIndex(index).child(0).asMObject()
    if geometry.isNull() or not geometry.hasFn(om.MFn.kMesh):
        raise ValueError("The skin's input geometry is not a mesh")
    bind_shape = _maya_matrix(_node_plug(skin, "geomMatrix"))
    pre_plug = _node_plug(skin, "bindPreMatrix")
    names, bind_pre, per_vertex = {}, {}, {}
    for path in fn.influenceObjects():
        name = convert.joint_name(path.partialPathName(), _joint_override(path))
        logical = fn.indexForInfluenceObject(path)
        components, weights = fn.getPointsAffectedByInfluence(path)
        affected = []
        if components.length():
            _path, component = components.getComponent(0)
            elements = om.MFnSingleIndexedComponent(component).getElements()
            affected = [(vertex, weight) for vertex, weight in zip(elements, weights) if weight > 0.0]
        if not affected:
            continue
        if name in bind_pre:
            raise ValueError("Several weighted joints map to the avatar joint " + name)
        if name not in supported:
            raise ValueError("Unsupported weighted joint: " + name)
        names[path.fullPathName()] = name
        bind_pre[name] = _maya_matrix(pre_plug.elementByLogicalIndex(logical))
        for vertex, weight in affected:
            per_vertex.setdefault(vertex, []).append((name, weight))
    return om.MFnMesh(geometry), bind_shape, bind_pre, per_vertex


def _rig_heads(skin):
    """SL joint name -> world position (Maya units) of every joint of the
    skin's skeleton, at bind time where the skin knows it: for the facing."""
    heads = {}
    selection = om.MSelectionList()
    selection.add(skin)
    fn = oma.MFnSkinCluster(selection.getDependNode(0))
    pre_plug = _node_plug(skin, "bindPreMatrix")
    roots = set()
    for path in fn.influenceObjects():
        name = convert.joint_name(path.partialPathName(), _joint_override(path))
        try:
            matrix = convert.inverse(_maya_matrix(pre_plug.elementByLogicalIndex(fn.indexForInfluenceObject(path))))
            heads[name] = convert.translation_of(matrix)
        except ValueError:
            pass
        root = path.fullPathName().split("|")
        roots.add("|".join(root[:2]))
    for root in roots:
        for joint in cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []:
            name = convert.joint_name(joint, cmds.getAttr(joint + ".auroraJoint") if cmds.attributeQuery(
                "auroraJoint", node=joint, exists=True) else None)
            if name not in heads:
                heads[name] = cmds.xform(joint, query=True, worldSpace=True, translation=True)
    return heads


def up_axis():
    return cmds.upAxis(query=True, axis=True)


def scene_unit(mode, transforms):
    """Metres per Maya unit: the chosen one, or found from the first rig of
    the synchronized meshes; real centimetres when no rig tells."""
    if mode in convert.UNITS:
        return convert.UNITS[mode]
    axis = up_axis()
    for transform in transforms:
        shape = shape_of(transform)
        skin = skin_of(shape) if shape else None
        if skin:
            try:
                unit = convert.rig_unit(_rig_heads(skin), axis)
            except (RuntimeError, ValueError):
                unit = None
            if unit is not None:
                return unit
    return convert.CM


def _mesh_arrays(fn, uv_set):
    counts, vertices = fn.getVertices()
    normal_counts, normal_ids = fn.getNormalIds()
    if list(normal_counts) != list(counts):
        raise ValueError("Inconsistent mesh normals")
    fv_uv = []
    if uv_set:
        uv_counts, uv_ids = fn.getAssignedUVs(uv_set)
        position = 0
        for count, uv_count in zip(counts, uv_counts):
            if uv_count == count:
                fv_uv.extend(uv_ids[position:position + count])
                position += count
            else:
                fv_uv.extend([-1] * count)
                position += uv_count
        u, v = fn.getUVs(uv_set)
        uvs = list(zip(u, v))
    else:
        fv_uv, uvs = [-1] * len(vertices), []
    triangle_counts, offsets = fn.getTriangleOffsets()
    triangles, polygons = convert.triangle_corners(list(triangle_counts), list(offsets), list(counts))
    return list(counts), list(vertices), list(normal_ids), fv_uv, uvs, triangles, polygons


def mesh(transform, supported, rest_axes, images, warnings, axes_mode=False, base=None):
    """One source as a wire mesh (without source_id or link). `base` takes
    Maya world to the SL frame (base_frame of the scene's up axis and unit)."""
    base = base if base is not None else convert.base_frame(up_axis(), convert.CM)
    shape = shape_of(transform)
    if shape is None:
        raise ValueError("No mesh shape")
    shape_path = _dag(shape)
    shape_fn = om.MFnMesh(shape_path)
    skin = skin_of(shape)
    if skin:
        fn, bind_shape, bind_pre, per_vertex = _skin(shape_path, skin, supported)
        if fn.numPolygons != shape_fn.numPolygons:
            raise ValueError("The skin's input mesh and the shape differ; delete non-deformer history")
        heads = {name: convert.point(base, head) for name, head in _rig_heads(skin).items()}
        frame = convert.multiply(convert.facing(heads), base)
        transform_matrix = convert.multiply(frame, bind_shape)
    else:
        fn, per_vertex, bind_pre = shape_fn, None, {}
        world = convert.from_maya(_values(shape_path.inclusiveMatrix()))
        # Rotation and scale stay; the world position is the preview's own.
        frame = base
        transform_matrix = convert.multiply(frame, convert.linear(world))
    points = fn.getPoints(om.MSpace.kObject)
    if len(points) == 0:
        raise ValueError("No vertices")
    positions = [convert.point(transform_matrix, (p.x, p.y, p.z)) for p in points]
    normal_transform = convert.normal_matrix(transform_matrix)
    normals = []
    for n in fn.getNormals(om.MSpace.kObject):
        x, y, z = convert.direction(normal_transform, (n.x, n.y, n.z))
        length = max((x * x + y * y + z * z) ** 0.5, 1e-20)
        normals.append((x / length, y / length, z / length))
    uv_set = shape_fn.currentUVSetName()
    try:
        counts, fv_vertex, fv_normal, fv_uv, uvs, triangles, polygons = _mesh_arrays(fn, uv_set)
    except RuntimeError:
        counts, fv_vertex, fv_normal, fv_uv, uvs, triangles, polygons = _mesh_arrays(fn, "")
    if len(fv_vertex) > convert.MAX_CORNERS:
        raise ValueError("more than 600,000 mesh corners")
    if not triangles:
        raise ValueError("no triangle surfaces")
    instance = shape_path.instanceNumber()
    groups, assignment = shape_fn.getConnectedShaders(instance)
    groups = [om.MFnDependencyNode(group).name() for group in groups]
    slots = [assignment[polygon] if polygon < len(assignment) else -1 for polygon in polygons]
    referenced = sorted({fv_vertex[corner] for triangle in triangles for corner in triangle})
    joints = weights = None
    names = []
    if per_vertex is not None:
        names = sorted({name for influences in per_vertex.values() for name, _weight in influences})
        if not names or len(names) > convert.MAX_JOINTS:
            raise ValueError("expected 1 to 110 weighted avatar joints")
        palette = {name: index for index, name in enumerate(names)}
        joints = [(0, 0, 0, 0)] * len(positions)
        weights = [(0.0, 0.0, 0.0, 0.0)] * len(positions)
        for vertex, influences in per_vertex.items():
            if vertex < len(positions):
                joints[vertex], weights[vertex] = convert.rank((palette[name], weight) for name, weight in influences)
        convert.complete(joints, weights, positions, referenced)
    flip = convert.determinant3(transform_matrix) < 0
    packed = convert.surfaces(positions, normals, uvs, fv_vertex, fv_normal, fv_uv, triangles, slots, flip, joints, weights)
    if not packed or len(packed) > convert.MAX_SURFACES:
        raise ValueError("expected 1 to 32 nonempty material surfaces")
    double_sided = bool(cmds.getAttr(shape + ".doubleSided"))
    faces = []
    for slot in sorted(packed):
        group = groups[slot] if 0 <= slot < len(groups) else None
        color, material = materials.capture(group, images, warnings, double_sided, bool(uvs))
        records, indices = packed[slot]
        faces.append({"color": color, "material": material, "_buffers": (records, indices)})
    inverse_bind = convert.inverse_binds(names, bind_pre, frame, axes_mode, rest_axes) if names else []
    name = transform.rsplit("|", 1)[-1]
    return {"name": name, "joints": names, "inverse_bind": inverse_bind, "faces": faces}, convert.corner_count(packed)


def link_offset(child, root, base):
    """The child origin minus the root origin in the SL frame, metres: the
    origins whose world position each static capture drops."""
    a = cmds.xform(child, query=True, worldSpace=True, matrix=True)[12:15]
    b = cmds.xform(root, query=True, worldSpace=True, matrix=True)[12:15]
    return convert.direction(base, [a[axis] - b[axis] for axis in range(3)])
