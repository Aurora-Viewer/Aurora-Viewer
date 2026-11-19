"""Receive the worn shape from Aurora as a new SL skeleton with the neutral
reference body skinned to it, in one undo step (Blender plug-in:
blender/aurora_live/shape_transfer.py, creation only).

The joints are created world-aligned in their own namespace, so their names
stay the SL names. Each joint keeps the shape's inverse bind and head
(`auroraShapeBind`, `auroraShapeHead`): a garment skinned to this skeleton
then previews with the shape once, not twice (capture.py reads them). Existing
nodes, weights and the live list are not touched.
"""

import json

import maya.api.OpenMaya as om
import maya.api.OpenMayaAnim as oma
from maya import cmds

from . import convert, shape_rules

ID_ATTRIBUTE = "auroraShapeId"


def existing(identity):
    """The group of a shape already received, by its transfer id."""
    for node in cmds.ls("*.{}".format(ID_ATTRIBUTE), objectsOnly=True, long=True, recursive=True) or []:
        if cmds.getAttr(node + "." + ID_ATTRIBUTE) == identity:
            return node
    return None


def _namespace(base):
    """A namespace for one received shape: a new one, or an empty one an
    undone transfer left behind."""
    name, index = base, 1
    while cmds.namespace(exists=":" + name):
        if not cmds.namespaceInfo(name, listNamespace=True):
            return name
        index += 1
        name = "{}{}".format(base, index)
    cmds.namespace(add=name)
    return name


def _object(path):
    selection = om.MSelectionList()
    selection.add(path)
    return selection.getDependNode(0)


def _check_joints(joints, targets, to_maya):
    """Every joint where the shape puts it: a skeleton off its reference
    body would preview wrong, so a mismatch stops the transfer."""
    size = max(abs(value) for row in convert.inverse(to_maya)[:3] for value in row[:3])
    tolerance = 1e-4 / max(size, 1e-9)  # 0.1 mm, in Maya units
    for name, path in joints.items():
        matrix = om.MDagPath.getAPathTo(_object(path)).inclusiveMatrix()
        position = (matrix[12], matrix[13], matrix[14])
        if max(abs(a - b) for a, b in zip(position, targets[name])) > tolerance:
            raise ValueError("The new skeleton did not land on the shape ({} is off); nothing was kept".format(name))


def _string_attribute(node, name, value):
    cmds.addAttr(node, longName=name, dataType="string")
    cmds.setAttr(node + "." + name, value, type="string")


def _array_attribute(node, name, values):
    cmds.addAttr(node, longName=name, dataType="doubleArray")
    cmds.setAttr(node + "." + name, [float(value) for value in values], type="doubleArray")


def _mesh(part, names, joints, to_maya, parent, namespace):
    """One reference body part as a skinned Maya mesh."""
    points = om.MPointArray([om.MPoint(*convert.point(to_maya, position)) for position in part["positions"]])
    counts = om.MIntArray([3] * len(part["triangles"]))
    connects = om.MIntArray([index for triangle in part["triangles"] for index in triangle])
    u = om.MFloatArray([float(uv[0]) for uv in part["uvs"]])
    v = om.MFloatArray([float(uv[1]) for uv in part["uvs"]])
    transform = om.MFnTransform()
    transform_object = transform.create(parent)
    transform.setName(namespace + ":body_" + part["name"])
    mesh = om.MFnMesh()
    mesh.create(points, counts, connects, u, v, transform_object)
    mesh.assignUVs(counts, connects)
    path = om.MDagPath.getAPathTo(transform_object).fullPathName()
    cmds.polySoftEdge(path, angle=180, constructionHistory=False)
    cmds.sets(path, edit=True, forceElement="initialShadingGroup")
    used = sorted({index for groups in part["weights"] for index, _weight in groups})
    influences = [joints[names[index]] for index in used]
    skin = cmds.skinCluster(influences, path, toSelectedBones=True, bindMethod=0, normalizeWeights=1,
                            maximumInfluences=4, obeyMaxInfluences=False,
                            name=namespace + ":skin_" + part["name"])[0]
    selection = om.MSelectionList()
    selection.add(skin)
    selection.add(path)
    fn = oma.MFnSkinCluster(selection.getDependNode(0))
    shape_path = selection.getDagPath(1)
    shape_path.extendToShape()
    order = {om.MDagPath(influence).fullPathName(): number for number, influence in enumerate(fn.influenceObjects())}
    columns = [order[cmds.ls(joints[names[index]], long=True)[0]] for index in used]
    column_of = dict(zip(used, columns))
    width = len(order)
    weights = om.MDoubleArray(len(part["positions"]) * width, 0.0)
    for vertex, groups in enumerate(part["weights"]):
        for index, weight in groups:
            weights[vertex * width + column_of[index]] += float(weight)
    component = om.MFnSingleIndexedComponent()
    vertices = component.create(om.MFn.kMeshVertComponent)
    component.addElements(list(range(len(part["positions"]))))
    fn.setWeights(shape_path, vertices, om.MIntArray(list(range(width))), weights, False)
    # Skinned at rest, the body must not have moved.
    placed = om.MFnMesh(shape_path).getPoints(om.MSpace.kWorld)
    tolerance = 1e-3 * max(1.0, max(abs(c) for p in points for c in (p.x, p.y, p.z)))
    if len(placed) != len(points) or any(max(abs(a.x - b.x), abs(a.y - b.y), abs(a.z - b.z)) > tolerance
                                         for a, b in zip(placed, points)):
        raise ValueError("The reference body moved when skinned ({}); nothing was kept".format(part["name"]))
    return path


def create(payload, up_axis, unit):
    """Build the shape's skeleton and reference body; returns its group."""
    joints_definition = shape_rules.skeleton()
    shape_rules.validate(payload, joints_definition)
    if payload.get("action", "create") != "create":
        raise ValueError("The Maya plug-in cannot apply a shape to an existing skeleton yet")
    found = existing(payload["id"])
    if found:
        return found
    to_maya = shape_rules.placement(up_axis, unit)
    names = payload["names"]
    selection = cmds.ls(selection=True, long=True) or []
    cmds.undoInfo(openChunk=True, chunkName="Aurora shape")
    namespace = group = None
    try:
        namespace = _namespace("AuroraShape")
        group = cmds.group(empty=True, world=True, name=namespace + ":" + shape_rules.node_name(payload["name"]))
        group = cmds.ls(group, long=True)[0]
        _string_attribute(group, ID_ATTRIBUTE, payload["id"])
        _string_attribute(group, "auroraShapeName", payload["name"])
        _string_attribute(group, "auroraShapeParameters", json.dumps(payload["params"], sort_keys=True))
        joints, objects, targets = {}, {}, {}
        group_object = _object(group)
        for (name, parent), joint in zip(joints_definition, payload["joints"]):
            # Joints stay world-aligned under an unmoved group, so each one's
            # translation is its head minus its parent's. OpenMaya takes
            # internal centimetres whatever the preferences show.
            target = convert.point(to_maya, joint["head"])
            local = [t - p for t, p in zip(target, targets[parent])] if parent else target
            fn = oma.MFnIkJoint()
            created = fn.create(objects[parent] if parent else group_object)
            fn.setName(namespace + ":" + name)
            fn.setTranslation(om.MVector(*local), om.MSpace.kTransform)
            objects[name], targets[name] = created, target
            joints[name] = om.MDagPath.getAPathTo(created).fullPathName()
            # Canonical metres in SL's frame, independent of Maya units.
            _array_attribute(joints[name], "auroraShapeBind", joint["inverse_bind"])
            _array_attribute(joints[name], "auroraShapeHead", joint["head"])
            if name[0].isupper():
                cmds.setAttr(joints[name] + ".drawStyle", 2)  # collision volumes: hidden
        _check_joints(joints, targets, to_maya)
        cmds.select(clear=True)
        bodies = cmds.group(empty=True, parent=group, name=namespace + ":reference_body")
        bodies = cmds.ls(bodies, long=True)[0]
        selection_list = om.MSelectionList()
        selection_list.add(bodies)
        parent = selection_list.getDependNode(0)
        for part in payload["parts"]:
            _mesh(part, names, joints, to_maya, parent, namespace)
        cmds.select(group, replace=True)
        return group
    except Exception:
        # Remove what was built; a cleanup failure must not hide the cause.
        try:
            if group and cmds.objExists(group):
                cmds.delete(group)
            if namespace and cmds.namespace(exists=":" + namespace):
                cmds.namespace(removeNamespace=":" + namespace, deleteNamespaceContent=True)
            if selection:
                cmds.select([node for node in selection if cmds.objExists(node)], replace=True)
        except Exception:
            pass
        raise
    finally:
        cmds.undoInfo(closeChunk=True)
