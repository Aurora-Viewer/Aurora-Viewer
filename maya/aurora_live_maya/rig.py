"""Create a Second Life rig in Maya, with an optional rigged test T-shirt,
and show or hide its parts through display layers (Blender plug-in:
blender/aurora_live/starter.py and the Skeleton visibility toggles).

The rig has all 133 Bento bones and 26 Fitted Mesh collision volumes of
Aurora's avatar_skeleton.xml at the standard SL rest proportions, world-
aligned in their own namespace (`AuroraRig`) so the joints keep their SL
names. Each part goes to a display layer (`SL_Body`, `SL_Bento_Hands`,
`SL_Bento_Face`, `SL_Fitted_Mesh`…): body and hands visible, the rest hidden.
Hiding a layer changes the display only, never deformation.
"""

from maya import cmds

from . import build, rig_rules, rigs, shape_rules

FABRIC = (0.34, 0.12, 0.72)


def layer_name(group):
    return "SL_" + group


def sort_layers(root, visible=None):
    """Put the rig's joints into the SL display layers (existing layers are
    reused). `visible` sets the starting visibility; None keeps it."""
    definition = {joint["name"]: joint for joint in rig_rules.default_joints()}
    members = {}
    for name, joint in rigs.joint_map(root).items():
        if name in definition:
            members.setdefault(layer_name(rig_rules.layer(definition[name])), []).append(joint)
    for layer, joints in members.items():
        if not cmds.objExists(layer):
            cmds.createDisplayLayer(name=layer, empty=True, noRecurse=True)
        cmds.editDisplayLayerMembers(layer, joints, noRecurse=True)
        if visible is not None:
            cmds.setAttr(layer + ".visibility", layer[3:] in visible)
    return sorted(members)


def layers():
    """(layer, visible) of every SL display layer in the scene."""
    return [(layer, bool(cmds.getAttr(layer + ".visibility")))
            for layer in sorted(cmds.ls("SL_*", type="displayLayer") or [])]


def show(layer, visible):
    cmds.setAttr(layer + ".visibility", bool(visible))


def _fabric(namespace):
    shader = cmds.shadingNode("lambert", asShader=True, name=namespace + ":Test_Fabric")
    cmds.setAttr(shader + ".color", *FABRIC, type="double3")
    group = cmds.sets(renderable=True, noSurfaceShader=True, empty=True, name=namespace + ":Test_FabricSG")
    cmds.connectAttr(shader + ".outColor", group + ".surfaceShader", force=True)
    return group


def create(up_axis, unit, with_tshirt=True):
    """A new SL rig at the origin, never replacing anything; returns
    (rig group, T-shirt or None)."""
    joints_definition = rig_rules.default_joints()
    to_maya = shape_rules.placement(up_axis, unit)
    with build.undoable("Aurora SL rig"):
        namespace = build.namespace("AuroraRig")
        group = cmds.ls(cmds.group(empty=True, world=True, name=namespace + ":SL_Avatar_Rig"), long=True)[0]
        build.string_attribute(group, "auroraStarter", "Second Life Bento / Fitted Mesh")
        joints = build.skeleton([(j["name"], j["parent"], j["head"]) for j in joints_definition], to_maya, group,
                                namespace + ":")
        sort_layers(joints["mPelvis"], visible=rig_rules.VISIBLE_LAYERS)
        shirt = None
        if with_tshirt:
            heads = {j["name"]: j["head"] for j in joints_definition}
            vertices, faces, uvs, weights = rig_rules.tshirt(heads)
            shirt = build.skinned_mesh(namespace + ":SL_Test_Tshirt", vertices, faces, uvs, weights, joints, to_maya,
                                       group, _fabric(namespace))
            cmds.select(shirt, replace=True)
        else:
            cmds.select(group, replace=True)
    return group, shirt

