"""Shared Maya builders: SL skeletons and skinned meshes made through
OpenMaya (internal centimetres whatever the preferences show) and checked
after building, and one-undo-step tool runs. Used by the shape transfer
(shape.py) and the rig tools (rig.py).
"""

import contextlib

import maya.api.OpenMaya as om
import maya.api.OpenMayaAnim as oma
from maya import cmds

from . import convert


@contextlib.contextmanager
def unrecorded():
    """Scratch work kept out of the undo queue (and so out of undo steps)."""
    state = cmds.undoInfo(query=True, stateWithoutFlush=True)
    cmds.undoInfo(stateWithoutFlush=False)
    try:
        yield
    finally:
        cmds.undoInfo(stateWithoutFlush=state)


def to_ui(value):
    """Internal centimetres to the preferences' length unit, for cmds."""
    return om.MDistance(value, om.MDistance.kCentimeters).asUnits(om.MDistance.uiUnit())


def from_ui(value):
    """The preferences' length unit to internal centimetres."""
    return om.MDistance(value, om.MDistance.uiUnit()).asCentimeters()


@contextlib.contextmanager
def undoable(name):
    """One undo step; a failure undoes what the step had done, so a tool
    never leaves half its work in the scene. The step always records one
    command first, so undoing a failure can never reach the user's own
    previous action."""
    if not cmds.undoInfo(query=True, state=True):
        raise ValueError("Turn Maya's undo on (Edit > Undo settings) before using this tool")
    cmds.undoInfo(openChunk=True, chunkName=name)
    try:
        cmds.delete(cmds.createNode("network", name="auroraLiveStep", skipSelect=True))
        yield
    except Exception:
        cmds.undoInfo(closeChunk=True)
        try:
            cmds.undo()
        except RuntimeError:
            pass
        raise
    cmds.undoInfo(closeChunk=True)


# ------------------------------------------------ undoable OpenMaya edits

PENDING = None  # the edit auroraLiveEdit runs next (aurora_live_command.py)


def _command():
    """Load the plug-in command that records OpenMaya edits for undo."""
    import os
    from . import aurora_live_command
    if not cmds.pluginInfo(aurora_live_command.NAME, query=True, loaded=True) and not hasattr(cmds, aurora_live_command.NAME):
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "aurora_live_command.py")
        cmds.loadPlugin(path, quiet=True)
    return getattr(cmds, aurora_live_command.NAME)


def edit(apply):
    """Run `apply` (which returns its revert function) as one undoable Maya
    command, so a fast OpenMaya edit undoes and redoes like the rest."""
    global PENDING
    command = _command()
    PENDING = apply
    try:
        command()
    finally:
        PENDING = None


def check_rest(root):
    """Refuse a rig posed away from its bind pose: tools that bind or
    reshape would bake the pose in."""
    poses = set()
    for joint in [root] + (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []):
        poses.update(cmds.listConnections(joint + ".bindPose", source=False, destination=True, type="dagPose") or [])
    for pose in poses:
        off = cmds.dagPose(pose, query=True, atPose=True) or []
        if off:
            raise ValueError("Put the rig in its bind pose first (Skin > Go to Bind Pose); "
                             "{} joints are posed".format(len(off)))


def namespace(base):
    """A new namespace, or an empty one an undone step left behind."""
    name, index = base, 1
    while cmds.namespace(exists=":" + name):
        if not cmds.namespaceInfo(name, listNamespace=True):
            return name
        index += 1
        name = "{}{}".format(base, index)
    cmds.namespace(add=name, parent=":")
    return name


def node_object(path):
    selection = om.MSelectionList()
    selection.add(path)
    return selection.getDependNode(0)


def string_attribute(node, name, value):
    if not cmds.attributeQuery(name, node=node, exists=True):
        cmds.addAttr(node, longName=name, dataType="string")
    cmds.setAttr(node + "." + name, value, type="string")


def array_attribute(node, name, values):
    if not cmds.attributeQuery(name, node=node, exists=True):
        cmds.addAttr(node, longName=name, dataType="doubleArray")
    cmds.setAttr(node + "." + name, [float(value) for value in values], type="doubleArray")


def world_position(path):
    matrix = om.MDagPath.getAPathTo(node_object(path)).inclusiveMatrix()
    return (matrix[12], matrix[13], matrix[14])


def skeleton(definition, to_maya, group, prefix):
    """World-aligned joints under `group` (an unmoved transform), from
    (name, parent, SL-metre head) in pre-order; checked to land where they
    should. Created with undoable commands. Returns SL name -> joint path."""
    joints, targets = {}, {}
    for name, parent, head in definition:
        # Each translation is the head minus its parent's: no joint turns.
        target = convert.point(to_maya, head)
        local = [t - p for t, p in zip(target, targets[parent])] if parent else target
        created = cmds.createNode("joint", name=prefix + name, parent=joints[parent] if parent else group,
                                  skipSelect=True)
        path = cmds.ls(created, long=True)[0]
        cmds.setAttr(path + ".translate", *[to_ui(value) for value in local])
        joints[name], targets[name] = path, target
        if name[0].isupper():
            cmds.setAttr(path + ".drawStyle", 2)  # collision volumes: hidden
    size = max(abs(value) for row in convert.inverse(to_maya)[:3] for value in row[:3])
    tolerance = 1e-4 / max(size, 1e-9)  # 0.1 mm, in Maya units
    for name, path in joints.items():
        if max(abs(a - b) for a, b in zip(world_position(path), targets[name])) > tolerance:
            raise ValueError("The new skeleton did not land where it should ({} is off); nothing was kept".format(name))
    return joints


def skinned_mesh(name, positions, polygons, uvs, weights, joints, to_maya, parent, shading_group="initialShadingGroup"):
    """A mesh skinned to `joints` at rest, from SL-metre positions, polygons
    (vertex index lists), UVs per polygon corner and per-vertex lists of (SL
    joint name, weight); checked not to move when skinned. Returns its path."""
    points = om.MPointArray([om.MPoint(*convert.point(to_maya, position)) for position in positions])
    counts = om.MIntArray([len(polygon) for polygon in polygons])
    connects = om.MIntArray([index for polygon in polygons for index in polygon])
    u = om.MFloatArray([float(uv[0]) for corners in uvs for uv in corners])
    v = om.MFloatArray([float(uv[1]) for corners in uvs for uv in corners])
    # The API builds meshes fast but outside the undo queue: build a scratch
    # copy unrecorded, then duplicate it with an undoable command.
    with unrecorded():
        transform = om.MFnTransform()
        scratch_object = transform.create()
        mesh = om.MFnMesh()
        mesh.create(points, counts, connects, u, v, scratch_object)
        mesh.assignUVs(counts, om.MIntArray(list(range(len(connects)))))
        scratch = om.MDagPath.getAPathTo(scratch_object).fullPathName()
    try:
        created = cmds.duplicate(scratch, name=name.rsplit(":", 1)[-1] + "_tmp")[0]
        created = cmds.parent(created, parent)[0]
        path = cmds.ls(cmds.rename(created, name), long=True)[0]
    finally:
        with unrecorded():
            if cmds.objExists(scratch):
                cmds.delete(scratch)
    cmds.polySoftEdge(path, angle=180, constructionHistory=False)
    cmds.sets(path, edit=True, forceElement=shading_group)
    used = sorted({joint for groups in weights for joint, _weight in groups})
    skin = cmds.skinCluster([joints[joint] for joint in used], path, toSelectedBones=True, bindMethod=0,
                            normalizeWeights=1, maximumInfluences=4, obeyMaxInfluences=False,
                            name=name + "_skin")[0]
    set_weights(skin, path, [[(joints[joint], weight) for joint, weight in groups] for groups in weights])
    placed = om.MFnMesh(_shape_path(path)).getPoints(om.MSpace.kWorld)
    tolerance = 1e-3 * max(1.0, max(abs(c) for p in points for c in (p.x, p.y, p.z)))
    if len(placed) != len(points) or any(max(abs(a.x - b.x), abs(a.y - b.y), abs(a.z - b.z)) > tolerance
                                         for a, b in zip(placed, points)):
        raise ValueError("The mesh moved when skinned ({}); nothing was kept".format(name))
    return path


def _skin_fn(skin):
    return oma.MFnSkinCluster(node_object(skin))


def _shape_path(mesh):
    path = om.MDagPath.getAPathTo(node_object(mesh))
    if path.hasFn(om.MFn.kTransform):
        path.extendToShape()
    return path


def influences(skin):
    """Joint paths of a skinCluster, in its weight column order."""
    return [path.fullPathName() for path in _skin_fn(skin).influenceObjects()]


def get_weights(skin, mesh):
    """Per vertex, the list of (influence path, weight > 0)."""
    fn = _skin_fn(skin)
    shape = _shape_path(mesh)
    component = om.MFnSingleIndexedComponent()
    vertices = component.create(om.MFn.kMeshVertComponent)
    count = om.MFnMesh(shape).numVertices
    component.addElements(list(range(count)))
    values, width = fn.getWeights(shape, vertices)
    values = list(values)
    names = influences(skin)
    result = []
    for row in range(count):
        chunk = values[row * width:(row + 1) * width]
        result.append([(names[column], value) for column, value in enumerate(chunk) if value > 0])
    return result


def set_weights(skin, mesh, weights):
    """Replace every vertex's weights with (influence path, weight) lists,
    in one undoable edit; influences not yet in the skin are added first."""
    names = influences(skin)
    missing = sorted({joint for groups in weights for joint, _weight in groups} - set(names))
    if missing:
        cmds.skinCluster(skin, edit=True, addInfluence=missing, weight=0.0)
        names = influences(skin)
    column = {name: index for index, name in enumerate(names)}
    width = len(names)
    values = om.MDoubleArray(len(weights) * width, 0.0)
    for row, groups in enumerate(weights):
        for joint, weight in groups:
            values[row * width + column[joint]] += float(weight)
    shape = _shape_path(mesh).fullPathName()

    def apply():
        fn = _skin_fn(skin)
        path = _shape_path(shape)
        component = om.MFnSingleIndexedComponent()
        vertices = component.create(om.MFn.kMeshVertComponent)
        component.addElements(list(range(len(weights))))
        every = om.MIntArray(list(range(width)))
        old = fn.setWeights(path, vertices, every, values, False, True)

        def revert():
            fn.setWeights(path, vertices, every, old, False)
        return revert

    edit(apply)


def set_points(mesh, points):
    """Replace a mesh's object-space points (internal centimetres) in one
    undoable edit."""
    shape = _shape_path(mesh).fullPathName()
    new = om.MPointArray([om.MPoint(*point) for point in points])

    def apply():
        fn = om.MFnMesh(_shape_path(shape))
        old = fn.getPoints(om.MSpace.kObject)
        fn.setPoints(new, om.MSpace.kObject)

        def revert():
            fn.setPoints(old, om.MSpace.kObject)
        return revert

    edit(apply)


def skin_of(mesh):
    from . import capture
    shape = capture.shape_of(mesh) if cmds.nodeType(mesh) == "transform" else mesh
    return capture.skin_of(shape) if shape else None
