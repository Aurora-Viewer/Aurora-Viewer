"""The shape's bind kept on a rig's joints (`auroraShapeBind`,
`auroraShapeHead`, SL metres): capture.py uses it so a garment skinned to a
reshaped rig previews with the shape once, as the Blender plug-in's
aurora_shape_bind. Plain maya.cmds, so every tool can use it."""

from maya import cmds


def _array(node, name, values):
    if not cmds.attributeQuery(name, node=node, exists=True):
        cmds.addAttr(node, longName=name, dataType="doubleArray")
    cmds.setAttr(node + "." + name, [float(value) for value in values], type="doubleArray")


def remember(payload, joints):
    """Record each transferred joint's bind on the rig joint of that name."""
    for joint in payload["joints"]:
        node = joints.get(joint["name"])
        if node is not None:
            _array(node, "auroraShapeBind", joint["inverse_bind"])
            _array(node, "auroraShapeHead", joint["head"])
