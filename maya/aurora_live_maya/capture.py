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


def dag(name):
    selection = om.MSelectionList()
    selection.add(name)
    return selection.getDagPath(0)


def shape_of(transform):
    """The visible mesh shape of a transform, or None."""
    shapes = cmds.listRelatives(transform, shapes=True, noIntermediate=True, fullPath=True, type="mesh") or []
    return shapes[0] if shapes else None


def skin_of(shape):
    """The skinCluster that deforms this shape itself, or None: the history
    also reaches the skins of other meshes (a wrap driven by a skinned body,
    a skinned blend shape target), which must not be taken for this one's."""
    target = dag(shape).fullPathName()
    for node in cmds.listHistory(shape, pruneDagObjects=True, interestLevel=1) or []:
        if cmds.nodeType(node) != "skinCluster":
            continue
        for geometry in cmds.skinCluster(node, query=True, geometry=True) or []:
            paths = cmds.ls(geometry, long=True) or []
            if paths and (paths[0] == target or target in (cmds.listRelatives(paths[0], shapes=True, fullPath=True) or [])):
                return node
    return None


def world_translation(node):
    """A node's world position in Maya's internal unit (centimetres), as
    OpenMaya returns every other value: cmds.xform answers in the unit of
    the preferences instead."""
    matrix = dag(node).inclusiveMatrix()
    return [matrix[12], matrix[13], matrix[14]]


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
    bind_pre, per_vertex = {}, {}
    for path in fn.influenceObjects():
        name = convert.joint_name(path.partialPathName(), _joint_override(path))
        logical = fn.indexForInfluenceObject(path)
        components, weights = fn.getPointsAffectedByInfluence(path)
        affected, offset = [], 0
        # One weight per listed vertex, item after item; a skin can deform
        # other shapes too, whose items are skipped.
        for item in range(components.length()):
            item_path, component = components.getComponent(item)
            elements = om.MFnSingleIndexedComponent(component).getElements()
            if item_path.fullPathName() == shape_path.fullPathName():
                affected += [(vertex, weight) for vertex, weight in zip(elements, weights[offset:offset + len(elements)])
                             if weight > 0.0]
            offset += len(elements)
        if not affected:
            continue
        if name in bind_pre:
            raise ValueError("Several weighted joints map to the avatar joint " + name)
        if name not in supported:
            raise ValueError("Unsupported weighted joint: " + name)
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
                heads[name] = world_translation(joint)
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


def _mesh_arrays(fn, uv_set, uv_fn=None):
    """Face-vertex arrays of a mesh. `uv_fn`, the output shape of a skinned
    mesh, gives the UVs: UV edits made after skinning live there, and the
    skin moves no UV."""
    uv_fn = uv_fn or fn
    counts, vertices = fn.getVertices()
    normal_counts, normal_ids = fn.getNormalIds()
    if list(normal_counts) != list(counts):
        raise ValueError("Inconsistent mesh normals")
    fv_uv = []
    if uv_set:
        uv_counts, uv_ids = uv_fn.getAssignedUVs(uv_set)
        position = 0
        for count, uv_count in zip(counts, uv_counts):
            if uv_count == count:
                fv_uv.extend(uv_ids[position:position + count])
                position += count
            else:
                fv_uv.extend([-1] * count)
                position += uv_count
        u, v = uv_fn.getUVs(uv_set)
        uvs = list(zip(u, v))
    else:
        fv_uv, uvs = [-1] * len(vertices), []
    triangle_counts, offsets = fn.getTriangleOffsets()
    triangles, polygons = convert.triangle_corners(list(triangle_counts), list(offsets), list(counts))
    return list(counts), list(vertices), list(normal_ids), fv_uv, uvs, triangles, polygons


class Source:
    """What a capture of one synchronized mesh needs from Maya: its geometry
    at rest, the matrix to the SL frame and, when skinned, its skin."""

    def __init__(self, transform, supported, base):
        self.transform = transform
        self.name = transform.rsplit("|", 1)[-1]
        self.shape = shape_of(transform)
        if self.shape is None:
            raise ValueError("No mesh shape")
        self.shape_path = dag(self.shape)
        self.shape_fn = om.MFnMesh(self.shape_path)
        self.skin = skin_of(self.shape)
        if self.skin:
            self.fn, bind_shape, self.bind_pre, self.per_vertex = _skin(self.shape_path, self.skin, supported)
            if self.fn.numPolygons != self.shape_fn.numPolygons:
                raise ValueError("The skin's input mesh and the shape differ; delete non-deformer history")
            heads = {name: convert.point(base, head) for name, head in _rig_heads(self.skin).items()}
            self.frame = convert.multiply(convert.facing(heads), base)
            self.matrix = convert.multiply(self.frame, bind_shape)
        else:
            self.fn, self.per_vertex, self.bind_pre = self.shape_fn, None, {}
            world = convert.from_maya(_values(self.shape_path.inclusiveMatrix()))
            # Rotation and scale stay; the world position is the preview's own.
            self.frame = base
            self.matrix = convert.multiply(base, convert.linear(world))


class Geometry:
    """Positions, normals, UVs, triangles and shading groups of a mesh,
    already in the SL frame."""

    def __init__(self, fn, shape_fn, shape_path, matrix):
        points = fn.getPoints(om.MSpace.kObject)
        if len(points) == 0:
            raise ValueError("No vertices")
        self.positions = [convert.point(matrix, (p.x, p.y, p.z)) for p in points]
        normal_transform = convert.normal_matrix(matrix)
        self.normals = []
        for n in fn.getNormals(om.MSpace.kObject):
            x, y, z = convert.direction(normal_transform, (n.x, n.y, n.z))
            length = max((x * x + y * y + z * z) ** 0.5, 1e-20)
            self.normals.append((x / length, y / length, z / length))
        uv_set = shape_fn.currentUVSetName()
        try:
            arrays = _mesh_arrays(fn, uv_set, shape_fn)
        except RuntimeError:
            arrays = _mesh_arrays(fn, "")
        _counts, self.fv_vertex, self.fv_normal, self.fv_uv, self.uvs, self.triangles, polygons = arrays
        # The viewer counts shared corners; this only stops hopeless meshes
        # early, pack() checks the real count.
        if len(self.fv_vertex) > 4 * convert.MAX_CORNERS:
            raise ValueError("more than 600,000 mesh corners")
        if not self.triangles:
            raise ValueError("no triangle surfaces")
        groups, assignment = shape_fn.getConnectedShaders(shape_path.instanceNumber())
        self.groups = [om.MFnDependencyNode(group).name() for group in groups]
        self.slots = [assignment[polygon] if polygon < len(assignment) else -1 for polygon in polygons]
        self.referenced = sorted({self.fv_vertex[corner] for triangle in self.triangles for corner in triangle})
        self.flip = convert.determinant3(matrix) < 0

    def pack(self, joints=None, weights=None):
        packed = convert.surfaces(self.positions, self.normals, self.uvs, self.fv_vertex, self.fv_normal, self.fv_uv,
                                  self.triangles, self.slots, self.flip, joints, weights)
        if not packed or len(packed) > convert.MAX_SURFACES:
            raise ValueError("expected 1 to 32 nonempty material surfaces")
        if convert.corner_count(packed) > convert.MAX_CORNERS:
            raise ValueError("more than 600,000 mesh corners")
        return packed


def skin_weights(per_vertex, geometry):
    """The SL joint palette and each vertex's strongest four, completed."""
    names = sorted({name for influences in per_vertex.values() for name, _weight in influences})
    if not names or len(names) > convert.MAX_JOINTS:
        raise ValueError("expected 1 to 110 weighted avatar joints")
    palette = {name: index for index, name in enumerate(names)}
    count = len(geometry.positions)
    joints = [(0, 0, 0, 0)] * count
    weights = [(0.0, 0.0, 0.0, 0.0)] * count
    for vertex, influences in per_vertex.items():
        if vertex < count:
            joints[vertex], weights[vertex] = convert.rank((palette[name], weight) for name, weight in influences)
    convert.complete(joints, weights, geometry.positions, geometry.referenced)
    return names, joints, weights


def faces(packed, geometry, shape, images, warnings):
    """Wire surfaces in slot order, and the shading group of each."""
    double_sided = bool(cmds.getAttr(shape + ".doubleSided"))
    result, groups = [], []
    for slot in sorted(packed):
        group = geometry.groups[slot] if 0 <= slot < len(geometry.groups) else None
        color, material = materials.capture(group, images, warnings, double_sided, bool(geometry.uvs))
        result.append({"color": color, "material": material, "_buffers": packed[slot]})
        groups.append(group)
    return result, groups


def mesh(transform, supported, rest_axes, images, warnings, axes_mode=False, base=None, groups=None):
    """One source as a wire mesh (without source_id or link). `base` takes
    Maya world to the SL frame (base_frame of the scene's up axis and unit).
    `groups`, a list, receives the shading group of each surface."""
    base = base if base is not None else convert.base_frame(up_axis(), convert.CM)
    source = Source(transform, supported, base)
    geometry = Geometry(source.fn, source.shape_fn, source.shape_path, source.matrix)
    names, joints, weights = skin_weights(source.per_vertex, geometry) if source.skin else ([], None, None)
    packed = geometry.pack(joints, weights)
    surfaces, surface_groups = faces(packed, geometry, source.shape, images, warnings)
    if groups is not None:
        groups.extend(surface_groups)
    inverse_bind = convert.inverse_binds(names, source.bind_pre, source.frame, axes_mode, rest_axes) if names else []
    return ({"name": source.name, "joints": names, "inverse_bind": inverse_bind, "faces": surfaces},
            convert.corner_count(packed))


def link_offset(child, root, base):
    """The child origin minus the root origin in the SL frame, metres: the
    origins whose world position each static capture drops."""
    a, b = world_translation(child), world_translation(root)
    return convert.direction(base, [a[axis] - b[axis] for axis in range(3)])
