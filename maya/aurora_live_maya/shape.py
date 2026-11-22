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

from maya import cmds

from . import build, shape_rules

ID_ATTRIBUTE = "auroraShapeId"


def existing(identity):
    """The group of a shape already received, by its transfer id."""
    for node in cmds.ls("*.{}".format(ID_ATTRIBUTE), objectsOnly=True, long=True, recursive=True) or []:
        if cmds.getAttr(node + "." + ID_ATTRIBUTE) == identity:
            return node
    return None


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
    with build.undoable("Aurora shape"):
        namespace = build.namespace("AuroraShape")
        group = cmds.group(empty=True, world=True, name=namespace + ":" + shape_rules.node_name(payload["name"]))
        group = cmds.ls(group, long=True)[0]
        build.string_attribute(group, ID_ATTRIBUTE, payload["id"])
        build.string_attribute(group, "auroraShapeName", payload["name"])
        build.string_attribute(group, "auroraShapeParameters", json.dumps(payload["params"], sort_keys=True))
        joints = build.skeleton([(name, parent, joint["head"]) for (name, parent), joint in
                                 zip(joints_definition, payload["joints"])], to_maya, group, namespace + ":")
        for joint in payload["joints"]:
            # Canonical metres in SL's frame, independent of Maya units.
            build.array_attribute(joints[joint["name"]], "auroraShapeBind", joint["inverse_bind"])
            build.array_attribute(joints[joint["name"]], "auroraShapeHead", joint["head"])
        bodies = cmds.ls(cmds.group(empty=True, parent=group, name=namespace + ":reference_body"), long=True)[0]
        for part in payload["parts"]:
            build.skinned_mesh(namespace + ":body_" + part["name"], part["positions"], part["triangles"],
                               [[part["uvs"][index] for index in triangle] for triangle in part["triangles"]],
                               [[(names[index], weight) for index, weight in groups] for groups in part["weights"]],
                               joints, to_maya, bodies)
        cmds.select(group, replace=True)
    return group
