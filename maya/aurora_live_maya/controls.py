"""Posing controls for an SL rig in Maya (Blender plug-in: Hand/foot IK,
Root / COG and look-at controls), with Maya's IK handles and constraints.

- Root: a ring on the ground under the pelvis; moves the whole posed rig.
- COG: a diamond at the pelvis under Root; the pelvis follows it.
- IK for each hand and foot: a target at the wrist or ankle and a pole
  (elbows behind the arms, knees ahead: rig_rules.pole_position). Off by
  default. Switching on snaps the target to the limb and lets its rotation
  turn the hand or foot; switching off leaves the limb as it is, in FK.
- Look at: a target for the classic eyes and one for the Bento eyes, two
  metres ahead (in the rig's scale). Focus turns the eyes toward it;
  switching focus off keeps the eyes where they look.

Constraints on SL joints exist only while IK or focus is on, so the joints
stay free to pose otherwise. Every control carries an `auroraControl` tag,
which finds it even when Maya renamed it. Controls are never skinned, so
captures ignore them; SL joints keep their rest, names and hierarchy, and
joints already driven by something else are refused. Each action is one
undo step; creating again reuses what exists.
"""

import math

import maya.api.OpenMaya as om
from maya import cmds

from . import build, capture, rig_rules, rigs

LEFT_COLOR, RIGHT_COLOR, CENTRE_COLOR = 18, 13, 17  # Maya colour indices: cyan, red, yellow
FOR_ATTRIBUTE = "auroraControlsFor"
TAG = "auroraControl"
EYES = {"Classic": ("mEyeLeft", "mEyeRight"), "Bento": ("mFaceEyeAltLeft", "mFaceEyeAltRight")}
DRIVEN = ("translate", "rotate", "translateX", "translateY", "translateZ", "rotateX", "rotateY", "rotateZ")


def _up():
    return (0.0, 1.0, 0.0) if capture.up_axis().lower() == "y" else (0.0, 0.0, 1.0)


def _dot_up(point):
    return sum(p * u for p, u in zip(point, _up()))


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


def _metre(heads):
    """One SL metre in the rig's scale: SL's pelvis stands about a metre above
    the feet (rig_rules.PELVIS_HEIGHT)."""
    return _dot_up(heads["mPelvis"]) - min(_dot_up(head) for head in heads.values())


# ------------------------------------------------------------- the group

def group_for(root, create=True):
    """The rig's controls group, found by its attribute."""
    for node in cmds.ls("*." + FOR_ATTRIBUTE, objectsOnly=True, long=True, recursive=True) or []:
        if cmds.getAttr(node + "." + FOR_ATTRIBUTE) == root:
            return node
    if not create:
        return None
    short = root.rsplit("|", 1)[-1].replace(":", "_")
    node = cmds.ls(cmds.group(empty=True, world=True, name=":" + short + "_AuroraControls"), long=True)[0]
    build.string_attribute(node, FOR_ATTRIBUTE, root)
    return node


def find(root, role):
    """A control of the rig by its tag (`root`, `cog`, `ik:HandLeft`,
    `pole:HandLeft`, `handle:HandLeft`, `look:Classic`), or None."""
    group = group_for(root, create=False)
    if group is None:
        return None
    for node in cmds.listRelatives(group, allDescendents=True, fullPath=True) or []:
        if cmds.attributeQuery(TAG, node=node, exists=True) and cmds.getAttr(node + "." + TAG) == role:
            return node
    return None


def _colour(node, index):
    for shape in cmds.listRelatives(node, shapes=True, fullPath=True) or []:
        cmds.setAttr(shape + ".overrideEnabled", True)
        cmds.setAttr(shape + ".overrideColor", index)


def _curve(name, role, kind, radius, parent, position, colour):
    """A tagged control curve; `radius` and `position` in internal cm."""
    radius = build.to_ui(radius)
    if kind == "diamond":
        node = cmds.circle(name=name, normal=_up(), radius=radius, degree=1, sections=4, constructionHistory=False)[0]
    elif kind == "cross":
        r = radius
        node = cmds.curve(name=name, degree=1, point=[(-r, 0, 0), (r, 0, 0), (0, 0, 0), (0, -r, 0), (0, r, 0),
                                                       (0, 0, 0), (0, 0, -r), (0, 0, r)])
    else:
        node = cmds.circle(name=name, normal=_up(), radius=radius, constructionHistory=False)[0]
    node = cmds.ls(cmds.parent(node, parent)[0], long=True)[0]
    cmds.xform(node, worldSpace=True, translation=[build.to_ui(v) for v in position])
    _colour(node, colour)
    build.string_attribute(node, TAG, role)
    return node


def _free(joint):
    """Refuse a joint whose position or rotation something already drives."""
    for attribute in DRIVEN:
        if cmds.listConnections(joint + "." + attribute, source=True, destination=False, skipConversionNodes=True):
            raise ValueError(joint.rsplit("|", 1)[-1] + " is already driven (a constraint, IK or animation); "
                             "remove that first")


def _rig(root=None):
    root = root or rigs.resolve()
    return root, rigs.joint_map(root)


def _holder(root, group):
    """Where new targets go: under Root when it exists."""
    return find(root, "root") or group


# --------------------------------------------------------------- Root / COG

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
        root_control = find(root, "root") or _curve("AuroraRoot", "root", "circle", size * 1.2, group, ground,
                                                    CENTRE_COLOR)
        if find(root, "cog") is None:
            _free(mapping["mPelvis"])
            cog = _curve("AuroraCOG", "cog", "diamond", size * 0.7, root_control, pelvis, CENTRE_COLOR)
            cmds.parentConstraint(cog, mapping["mPelvis"], maintainOffset=True, name="AuroraCOG_pelvis")
        # IK targets, poles and look targets made earlier move with Root.
        for node in cmds.listRelatives(group, children=True, fullPath=True) or []:
            if node != root_control and cmds.attributeQuery(TAG, node=node, exists=True):
                cmds.parent(node, root_control)
    return root_control


# --------------------------------------------------------------------- IK

def _preferred_bend(middle, root_head, middle_head, end_head, pole):
    """A small bend toward the pole as the middle joint's preferred angle, so
    the solver never meets a straight, undecided limb (exact at rest)."""
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
    """Hand and foot IK, switched off; returns the limbs' targets."""
    root, mapping = _rig(root)
    heads = _heads(mapping)
    result = {}
    with build.undoable("Aurora IK controls"):
        group = group_for(root)
        holder = _holder(root, group)
        for limb, chain in rig_rules.LIMBS.items():
            existing = find(root, "ik:" + limb)
            if existing:
                result[limb] = existing
                continue
            if not all(name in mapping for name in chain):
                raise ValueError("This rig lacks the joints of " + limb)
            for joint in chain:
                _free(mapping[joint])
            start, middle, end = (mapping[name] for name in chain)
            pole_position = rig_rules.pole_position(heads, limb, _up())
            _preferred_bend(middle, heads[chain[0]], heads[chain[1]], heads[chain[2]], pole_position)
            colour = LEFT_COLOR if limb.endswith("Left") else RIGHT_COLOR
            reach = _length(_sub(heads[chain[1]], heads[chain[0]]))
            target = _curve("AuroraIK_" + limb, "ik:" + limb, "circle", reach * 0.3, holder, heads[chain[2]], colour)
            pole = _curve("AuroraPole_" + limb, "pole:" + limb, "cross", reach * 0.12, holder, pole_position, colour)
            handle = cmds.ikHandle(startJoint=start, endEffector=end, solver="ikRPsolver",
                                   name="AuroraIKHandle_" + limb)[0]
            handle = cmds.ls(cmds.parent(handle, target)[0], long=True)[0]
            build.string_attribute(handle, TAG, "handle:" + limb)
            cmds.setAttr(handle + ".visibility", False)
            cmds.setAttr(handle + ".ikBlend", 0.0)
            cmds.poleVectorConstraint(pole, handle, name="AuroraPole_" + limb + "_constraint")
            result[limb] = target
    return result


def _orient(end):
    """The Aurora orient constraint on a limb's end joint, if any."""
    found = cmds.listConnections(end, type="orientConstraint", source=True, destination=False) or []
    return [node for node in set(found) if node.rsplit("|", 1)[-1].split(":")[-1].startswith("AuroraIK_")]


def set_ik(root, limb, enabled):
    """Switch one limb to IK (target snapped to the limb, its rotation
    turning the hand or foot) or back to FK as it stands."""
    target, handle = find(root, "ik:" + limb), find(root, "handle:" + limb)
    if target is None or handle is None:
        raise ValueError("Create the IK controls first")
    end = rigs.joint_map(root)[rig_rules.LIMBS[limb][2]]
    with build.undoable("Aurora IK " + ("on" if enabled else "off")):
        if enabled:
            cmds.matchTransform(target, end, position=True, rotation=True)
            if not _orient(end):
                cmds.orientConstraint(target, end, maintainOffset=True, name="AuroraIK_" + limb + "_orient")
            cmds.setAttr(handle + ".ikBlend", 1.0)
        else:
            # Keep the visible pose: the joint keeps its last rotation.
            for constraint in _orient(end):
                cmds.delete(constraint)
            cmds.setAttr(handle + ".ikBlend", 0.0)


def ik_state(root):
    """limb -> enabled, for the limbs that have IK."""
    result = {}
    for limb in rig_rules.LIMBS:
        handle = find(root, "handle:" + limb)
        if handle:
            result[limb] = cmds.getAttr(handle + ".ikBlend") > 0.5
    return result


# ---------------------------------------------------------------- look at

def look_at(root=None, which="Classic"):
    """A look-at target for one pair of eyes, focus off."""
    root, mapping = _rig(root)
    pair = EYES[which]
    if not all(name in mapping for name in pair):
        raise ValueError("This rig has no {} eyes".format(which.lower()))
    heads = _heads(mapping)
    with build.undoable("Aurora look-at target"):
        existing = find(root, "look:" + which)
        if existing:
            return existing
        for eye in pair:
            _free(mapping[eye])
        group = group_for(root)
        middle = tuple(sum(heads[eye][axis] for eye in pair) / 2 for axis in range(3))
        ahead = tuple(m + f * 2.0 * _metre(heads) for m, f in zip(middle, _forward(heads)))
        return _curve("AuroraLook_" + which, "look:" + which, "cross",
                      _length(_sub(heads[pair[0]], heads[pair[1]])) * 0.6, _holder(root, group), ahead, CENTRE_COLOR)


def _aims(mapping, which):
    result = []
    for eye in EYES[which]:
        if eye in mapping:
            found = cmds.listConnections(mapping[eye], type="aimConstraint", source=True, destination=False) or []
            result += [node for node in set(found) if node.rsplit("|", 1)[-1].split(":")[-1].startswith("AuroraLook_")]
    return result


def set_focus(root, which, enabled):
    """Turn the eyes toward the target, or leave them where they look."""
    mapping = rigs.joint_map(root)
    target = find(root, "look:" + which)
    if target is None:
        raise ValueError("Create the {} look-at target first".format(which.lower()))
    with build.undoable("Aurora focus " + ("on" if enabled else "off")):
        if enabled:
            if not _aims(mapping, which):
                for eye in EYES[which]:
                    cmds.aimConstraint(target, mapping[eye], maintainOffset=True, worldUpType="none",
                                       name="AuroraLook_" + which + "_" + eye)
        else:
            for constraint in _aims(mapping, which):
                cmds.delete(constraint)


def focus_state(root, which):
    """True or False when the pair has a target, None otherwise."""
    if find(root, "look:" + which) is None:
        return None
    return bool(_aims(rigs.joint_map(root), which))


def recenter(root, which):
    """Put the target back in front of the eyes, two metres ahead."""
    mapping = rigs.joint_map(root)
    target = find(root, "look:" + which)
    if target is None:
        raise ValueError("Create the {} look-at target first".format(which.lower()))
    heads = _heads(mapping)
    middle = tuple(sum(heads[eye][axis] for eye in EYES[which]) / 2 for axis in range(3))
    ahead = tuple(m + f * 2.0 * _metre(heads) for m, f in zip(middle, _forward(heads)))
    with build.undoable("Aurora recentre look-at"):
        cmds.xform(target, worldSpace=True, translation=[build.to_ui(v) for v in ahead])
