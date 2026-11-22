"""Posing controls for an SL rig in Maya (Blender plug-in: Hand/foot IK,
Root / COG and look-at controls), with Maya's IK handles and constraints.

- Root: a ring on the ground under the pelvis; moves the whole posed rig.
- COG: a diamond at the pelvis under Root; the pelvis follows it.
- IK for each hand and foot: a target at the wrist or ankle (its rotation
  turns the hand or foot) and a pole (elbows behind the arms, knees ahead,
  placed by rig_rules.pole_position). Off by default; switching on snaps the
  target to the limb.
- Look at: a target for the classic eyes and one for the Bento eyes, two
  metres ahead (scaled to the rig). Focus starts off, keeping eye poses.

Controls live in their own group, never in a skin, so captures ignore them.
SL joints keep their rest, names and hierarchy; joints already driven by a
constraint are refused. Each creation is one undo step and repeating it
reuses what exists.
"""

import math

import maya.api.OpenMaya as om
from maya import cmds

from . import build, capture, rig_rules, rigs

LEFT_COLOR, RIGHT_COLOR, CENTRE_COLOR = 18, 13, 17  # Maya colour indices: cyan, red, yellow
FOR_ATTRIBUTE = "auroraControlsFor"


def _up():
    return (0.0, 1.0, 0.0) if capture.up_axis().lower() == "y" else (0.0, 0.0, 1.0)


def _heads(mapping):
    return {name: build.world_position(path) for name, path in mapping.items()}


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _length(a):
    return math.sqrt(sum(x * x for x in a))


def _forward(heads):
    left, right = heads["mShoulderLeft"], heads["mShoulderRight"]
    across = _sub(left, right)
    up = _up()
    forward = (across[1] * up[2] - across[2] * up[1], across[2] * up[0] - across[0] * up[2],
               across[0] * up[1] - across[1] * up[0])
    length = _length(forward)
    if length < 1e-9:
        raise ValueError("Cannot tell which way the rig faces")
    return tuple(x / length for x in forward)


def group_for(root, create=True):
    """The rig's controls group, found by its attribute."""
    for node in cmds.ls("*." + FOR_ATTRIBUTE, objectsOnly=True, long=True, recursive=True) or []:
        if cmds.getAttr(node + "." + FOR_ATTRIBUTE) == root:
            return node
    if not create:
        return None
    short = root.rsplit("|", 1)[-1].replace(":", "_")
    node = cmds.ls(cmds.group(empty=True, world=True, name=short + "_AuroraControls"), long=True)[0]
    build.string_attribute(node, FOR_ATTRIBUTE, root)
    return node


def _child(group, name):
    found = [node for node in cmds.listRelatives(group, allDescendents=True, fullPath=True) or []
             if node.rsplit("|", 1)[-1].endswith(name)]
    return found[0] if found else None


def _colour(node, index):
    for shape in cmds.listRelatives(node, shapes=True, fullPath=True) or []:
        cmds.setAttr(shape + ".overrideEnabled", True)
        cmds.setAttr(shape + ".overrideColor", index)


def _curve(name, kind, radius, parent, position, colour):
    up = _up()
    normal = up
    if kind == "diamond":
        node = cmds.circle(name=name, normal=normal, radius=radius, degree=1, sections=4, constructionHistory=False)[0]
    elif kind == "cross":
        r = radius
        node = cmds.curve(name=name, degree=1, point=[(-r, 0, 0), (r, 0, 0), (0, 0, 0), (0, -r, 0), (0, r, 0), (0, 0, 0),
                                                       (0, 0, -r), (0, 0, r)])
    else:
        node = cmds.circle(name=name, normal=normal, radius=radius, constructionHistory=False)[0]
    node = cmds.ls(cmds.parent(node, parent)[0], long=True)[0]
    cmds.xform(node, worldSpace=True, translation=[build.to_ui(v) for v in position])
    _colour(node, colour)
    return node


def _free(joint):
    if cmds.listConnections(joint, type="constraint", source=True, destination=False):
        raise ValueError(joint.rsplit("|", 1)[-1] + " is already driven by a constraint; remove it first")


def _rig(root=None):
    root = root or rigs.resolve()
    return root, rigs.joint_map(root)


def root_cog(root=None):
    """Root and COG controls; the pelvis follows COG."""
    root, mapping = _rig(root)
    heads = _heads(mapping)
    pelvis = heads["mPelvis"]
    lowest = min(_dot_up(head) for head in heads.values())
    ground = tuple(p - u * (_dot_up(pelvis) - lowest) for p, u in zip(pelvis, _up()))
    size = _length(_sub(heads["mShoulderLeft"], heads["mShoulderRight"]))
    with build.undoable("Aurora root and COG"):
        group = group_for(root)
        root_control = _child(group, "AuroraRoot")
        if root_control is None:
            root_control = _curve("AuroraRoot", "circle", size * 1.2, group, ground, CENTRE_COLOR)
        cog = _child(group, "AuroraCOG")
        if cog is None:
            _free(mapping["mPelvis"])
            cog = _curve("AuroraCOG", "diamond", size * 0.7, root_control, pelvis, CENTRE_COLOR)
            cmds.parentConstraint(cog, mapping["mPelvis"], maintainOffset=True, name="AuroraCOG_pelvis")
        # IK and look targets made earlier move with Root.
        for name in ("AuroraIK", "AuroraLook"):
            for node in cmds.listRelatives(group, children=True, fullPath=True) or []:
                if node.rsplit("|", 1)[-1].startswith(name):
                    cmds.parent(node, root_control)
    return root_control, cog


def _dot_up(point):
    return sum(p * u for p, u in zip(point, _up()))


def _holder(group):
    """Where new targets go: under Root when it exists."""
    return _child(group, "AuroraRoot") or group


def _preferred_bend(middle, root_head, middle_head, end_head, pole):
    """A small bend toward the pole as the middle joint's preferred angle, so
    the solver never meets a straight, undecided limb."""
    limb = _sub(end_head, root_head)
    toward = _sub(pole, middle_head)
    axis = (limb[1] * toward[2] - limb[2] * toward[1], limb[2] * toward[0] - limb[0] * toward[2],
            limb[0] * toward[1] - limb[1] * toward[0])
    length = _length(axis)
    if length < 1e-9:
        return
    matrix = om.MDagPath.getAPathTo(build.node_object(middle)).inclusiveMatrixInverse()
    local = om.MVector(*[a / length for a in axis]) * matrix
    local.normalize()
    angle = -10.0  # degrees, small: the pole sets the plane
    cmds.setAttr(middle + ".preferredAngle", local.x * angle, local.y * angle, local.z * angle)


def ik(root=None):
    """Hand and foot IK, switched off; returns the four limbs' targets."""
    root, mapping = _rig(root)
    heads = _heads(mapping)
    up = _up()
    result = {}
    with build.undoable("Aurora IK controls"):
        group = group_for(root)
        holder = _holder(group)
        for limb, chain in rig_rules.LIMBS.items():
            if _child(group, "AuroraIK_" + limb):
                continue
            if not all(name in mapping for name in chain):
                raise ValueError("This rig lacks the joints of " + limb)
            for joint in chain:
                _free(mapping[joint])
            start, middle, end = (mapping[name] for name in chain)
            pole_position = rig_rules.pole_position(heads, limb, up)
            _preferred_bend(middle, heads[chain[0]], heads[chain[1]], heads[chain[2]], pole_position)
            colour = LEFT_COLOR if limb.endswith("Left") else RIGHT_COLOR
            reach = _length(_sub(heads[chain[1]], heads[chain[0]]))
            target = _curve("AuroraIK_" + limb, "circle", reach * 0.3, holder, heads[chain[2]], colour)
            pole = _curve("AuroraPole_" + limb, "cross", reach * 0.12, holder, pole_position, colour)
            handle = cmds.ikHandle(startJoint=start, endEffector=end, solver="ikRPsolver",
                                   name="AuroraIKHandle_" + limb)[0]
            handle = cmds.ls(cmds.parent(handle, target)[0], long=True)[0]
            cmds.setAttr(handle + ".visibility", False)
            cmds.poleVectorConstraint(pole, handle, name="AuroraPole_" + limb + "_constraint")
            cmds.orientConstraint(target, end, maintainOffset=True, name="AuroraIK_" + limb + "_orient")
            set_ik(root, limb, False)
            result[limb] = target
    return result


def _limb_nodes(root, limb):
    group = group_for(root, create=False)
    target = _child(group, "AuroraIK_" + limb) if group else None
    if target is None:
        return None
    handle = _child(target, "AuroraIKHandle_" + limb)
    end = rigs.joint_map(root)[rig_rules.LIMBS[limb][2]]
    orient = _constraints(end, "orientConstraint", "AuroraIK_")
    return target, handle, orient


def _constraints(joint, kind, prefix):
    """This joint's own Aurora constraints of a kind (not another rig's)."""
    found = cmds.listConnections(joint, type=kind, source=True, destination=False) or []
    return sorted({node for node in found if node.rsplit("|", 1)[-1].split(":")[-1].startswith(prefix)})


def set_ik(root, limb, enabled):
    """Switch one limb between IK and FK; switching on snaps the target to the
    limb's current wrist or ankle."""
    nodes = _limb_nodes(root, limb)
    if nodes is None:
        raise ValueError("Create the IK controls first")
    target, handle, orient = nodes
    end = rigs.joint_map(root)[rig_rules.LIMBS[limb][2]]
    if enabled:
        cmds.matchTransform(target, end, position=True)
    if handle:
        cmds.setAttr(handle + ".ikBlend", 1.0 if enabled else 0.0)
    for constraint in orient:
        for alias in cmds.orientConstraint(constraint, query=True, weightAliasList=True) or []:
            cmds.setAttr(constraint + "." + alias, 1.0 if enabled else 0.0)


def ik_state(root):
    """limb -> enabled, for the limbs that have IK."""
    result = {}
    for limb in rig_rules.LIMBS:
        nodes = _limb_nodes(root, limb)
        if nodes and nodes[1]:
            result[limb] = cmds.getAttr(nodes[1] + ".ikBlend") > 0.5
    return result


EYES = {"Classic": ("mEyeLeft", "mEyeRight"), "Bento": ("mFaceEyeAltLeft", "mFaceEyeAltRight")}


def look_at(root=None, which="Classic"):
    """A look-at target for one pair of eyes, focus off."""
    root, mapping = _rig(root)
    pair = EYES[which]
    if not all(name in mapping for name in pair):
        raise ValueError("This rig has no {} eyes".format(which.lower()))
    heads = _heads(mapping)
    with build.undoable("Aurora look-at target"):
        group = group_for(root)
        existing = _child(group, "AuroraLook_" + which)
        if existing:
            return existing
        for eye in pair:
            _free(mapping[eye])
        middle = tuple(sum(heads[eye][axis] for eye in pair) / 2 for axis in range(3))
        # Two metres in the rig's own scale: SL's pelvis stands about a metre
        # above the feet (rig_rules.PELVIS_HEIGHT).
        metre = _dot_up(heads["mPelvis"]) - min(_dot_up(head) for head in heads.values())
        ahead = tuple(m + f * 2.0 * metre for m, f in zip(middle, _forward(heads)))
        target = _curve("AuroraLook_" + which, "cross", _length(_sub(heads[pair[0]], heads[pair[1]])) * 0.6,
                        _holder(group), ahead, CENTRE_COLOR)
        for eye in pair:
            constraint = cmds.aimConstraint(target, mapping[eye], maintainOffset=True, worldUpType="none",
                                            name="AuroraLook_" + which + "_" + eye.rsplit("|", 1)[-1].split(":")[-1])[0]
            for alias in cmds.aimConstraint(constraint, query=True, weightAliasList=True) or []:
                cmds.setAttr(constraint + "." + alias, 0.0)
    return target


def set_focus(root, which, enabled):
    mapping = rigs.joint_map(root)
    for eye in EYES[which]:
        for constraint in _constraints(mapping[eye], "aimConstraint", "AuroraLook_") if eye in mapping else []:
            for alias in cmds.aimConstraint(constraint, query=True, weightAliasList=True) or []:
                cmds.setAttr(constraint + "." + alias, 1.0 if enabled else 0.0)


def focus_state(root, which):
    """True or False when the pair has a target, None otherwise."""
    mapping = rigs.joint_map(root)
    for eye in EYES[which]:
        for constraint in _constraints(mapping[eye], "aimConstraint", "AuroraLook_") if eye in mapping else []:
            for alias in cmds.aimConstraint(constraint, query=True, weightAliasList=True) or []:
                return cmds.getAttr(constraint + "." + alias) > 0.5
    return None


def recenter(root, which):
    """Put the target back in front of the eyes, two metres ahead of the
    current pose."""
    mapping = rigs.joint_map(root)
    group = group_for(root, create=False)
    target = _child(group, "AuroraLook_" + which) if group else None
    if target is None:
        raise ValueError("Create the {} look-at target first".format(which.lower()))
    heads = _heads(mapping)
    middle = tuple(sum(heads[eye][axis] for eye in EYES[which]) / 2 for axis in range(3))
    metre = _dot_up(heads["mPelvis"]) - min(_dot_up(head) for head in heads.values())
    ahead = tuple(m + f * 2.0 * metre for m, f in zip(middle, _forward(heads)))
    cmds.xform(target, worldSpace=True, translation=[build.to_ui(v) for v in ahead])
