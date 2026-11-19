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
    name, index = base, 1
    while cmds.namespace(exists=":" + name):
        index += 1
        name = "{}{}".format(base, index)
    cmds.namespace(add=name)
    return name


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
        joints = {}
        for (name, parent), joint in zip(joints_definition, payload["joints"]):
            # A joint is created under the selected joint; the root goes
            # under the group afterwards, keeping its world position.
            if parent:
                cmds.select(joints[parent], replace=True)
            else:
                cmds.select(clear=True)
            created = cmds.joint(name=namespace + ":" + name, position=convert.point(to_maya, joint["head"]),
                                 absolute=True)
            if not parent:
                created = cmds.parent(created, group)[0]
            joints[name] = cmds.ls(created, long=True)[0]
            # Canonical metres in SL's frame, independent of Maya units.
            _array_attribute(joints[name], "auroraShapeBind", joint["inverse_bind"])
            _array_attribute(joints[name], "auroraShapeHead", joint["head"])
            if name[0].isupper():
                cmds.setAttr(joints[name] + ".drawStyle", 2)  # collision volumes: hidden
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
        if group and cmds.objExists(group):
            cmds.delete(group)
        if namespace and cmds.namespace(exists=":" + namespace):
            cmds.namespace(removeNamespace=":" + namespace, deleteNamespaceContent=True)
        if selection:
            cmds.select([node for node in selection if cmds.objExists(node)], replace=True)
        raise
    finally:
        cmds.undoInfo(closeChunk=True)
