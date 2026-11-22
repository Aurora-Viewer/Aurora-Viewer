"""Apply a shape from Aurora to a plain SL rig in Maya, without MayaStar
(Blender plug-in: blender/aurora_live/shape_apply.py, generic rigs).

Each SL joint's change is its new world matrix times its previous inverse
bind: the shape's own inverse bind replaces the previous one (the rig's
last received shape, or SL's default skeleton). In Maya's frame:

    D = F^-1 · inverse(new inverse bind) · previous inverse bind · F

Joints move their heads by D (orientations stay). The rest geometry of every
mesh skinned to the rig moves by its vertices' blend of D, then each skin's
bind matrices take the joints' new place, so the meshes follow the new
proportions at rest. Joints and weights are kept. One undo step. Meshes with
blend shapes or other deformers before their skin are refused, as their
shapes would also need moving.
"""

import maya.api.OpenMaya as om
from maya import cmds

from . import bindinfo, build, convert, rigs, shape_rules


def _previous(joint, payload_joint):
    if cmds.attributeQuery("auroraShapeBind", node=joint, exists=True):
        values = cmds.getAttr(joint + ".auroraShapeBind") or []
        if len(values) == 16:
            return values
    return payload_joint["reference_bind"]


def _orig(skin):
    """The rest shape feeding a skin, refusing deformers before it."""
    source = (cmds.listConnections(skin + ".input[0].inputGeometry", source=True, destination=False, shapes=True)
              or [None])[0]
    if source and cmds.nodeType(source) == "tweak":
        source = (cmds.listConnections(source + ".input[0].inputGeometry", source=True, destination=False,
                                       shapes=True) or [None])[0]
    if not source or cmds.nodeType(source) != "mesh":
        raise ValueError("{}: blend shapes or other deformers come before its skin; "
                         "apply the shape with MayaStar or the Blender plug-in".format(skin))
    return cmds.ls(source, long=True)[0]


def _skins(root):
    """Every skinCluster whose influences belong to this rig."""
    joints = set([root] + (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []))
    result = []
    for skin in cmds.ls(type="skinCluster") or []:
        influences = cmds.ls(cmds.skinCluster(skin, query=True, influence=True) or [], long=True)
        if influences and set(influences) <= joints:
            result.append(skin)
    return result


def deltas(payload, mapping, frame):
    """SL name -> D in Maya's frame, for the rig's joints the shape names."""
    result = {}
    for joint in payload["joints"]:
        node = mapping.get(joint["name"])
        if node is None:
            continue
        result[joint["name"]] = convert.shape_delta(joint["inverse_bind"], _previous(node, joint), frame)
    return result


def _blend(groups, by_influence):
    """The weighted sum of the influences' matrices (identity for none)."""
    total = sum(weight for _influence, weight in groups)
    if total <= 1e-9:
        return convert.identity()
    result = [[0.0] * 4 for _ in range(4)]
    for influence, weight in groups:
        matrix = by_influence.get(influence, convert.identity())
        for row in range(4):
            for column in range(4):
                result[row][column] += matrix[row][column] * weight / total
    return result


def apply(payload, root, up_axis, unit):
    """Reshape a plain SL rig and its skinned meshes; returns the rig."""
    shape_rules.validate(payload, shape_rules.skeleton())
    mapping = rigs.joint_map(root)
    if not set(rigs.CORE).issubset(mapping):
        raise ValueError("The selected skeleton is not a Second Life rig")
    heads = {name: build.world_position(path) for name, path in mapping.items()}
    base = convert.base_frame(up_axis, unit)
    frame = convert.multiply(convert.facing({name: convert.point(base, head) for name, head in heads.items()}), base)
    moves = deltas(payload, mapping, frame)
    # A joint the shape does not name follows its nearest named ancestor.
    by_joint = {}
    for name, path in mapping.items():
        current, matrix = path, None
        while current and matrix is None:
            matrix = moves.get(rigs.sl_name(current)) if rigs.sl_name(current) in mapping else None
            parent = cmds.listRelatives(current, parent=True, fullPath=True, type="joint")
            current = parent[0] if parent else None
        by_joint[path] = matrix or convert.identity()
    skins = _skins(root)
    plans = []
    for skin in skins:
        orig = _orig(skin)
        geometry = convert.from_maya(cmds.getAttr(skin + ".geomMatrix"))
        mesh = (cmds.skinCluster(skin, query=True, geometry=True) or [None])[0]
        weights = build.get_weights(skin, mesh)
        points = om.MFnMesh(build._shape_path(orig)).getPoints(om.MSpace.kObject)
        if len(points) != len(weights):
            raise ValueError("{}: its rest mesh and its weights disagree".format(skin))
        moved = []
        geometry_inverse = convert.inverse(geometry)
        for point, groups in zip(points, weights):
            world = convert.point(geometry, (point.x, point.y, point.z))
            target = convert.point(_blend(groups, by_joint), world)
            moved.append(convert.point(geometry_inverse, target))
        plans.append((skin, orig, points, moved))
    with build.undoable("Aurora apply shape"):
        # Parents first: a child keeps its own new head after its parent moved.
        for name, path in sorted(mapping.items(), key=lambda item: item[1].count("|")):
            new_head = convert.point(by_joint[path], heads[name])
            cmds.xform(path, worldSpace=True, translation=[build.to_ui(v) for v in new_head])
        for skin, orig, points, moved in plans:
            # Rest points through the orig shape's tweaks (pnts), undoable.
            for index, (old, new) in enumerate(zip(points, moved)):
                offset = (new[0] - old.x, new[1] - old.y, new[2] - old.z)
                if max(abs(v) for v in offset) < 1e-9:
                    continue
                current = cmds.getAttr("{}.pnts[{}]".format(orig, index))[0]
                cmds.setAttr("{}.pnts[{}]".format(orig, index),
                             *[c + build.to_ui(o) for c, o in zip(current, offset)])
            # The skin binds at the joints' new place.
            fn_paths = build.influences(skin)
            for path in fn_paths:
                logical = _logical(skin, path)
                cmds.setAttr("{}.bindPreMatrix[{}]".format(skin, logical),
                             cmds.getAttr(path + ".worldInverseMatrix[0]"), type="matrix")
            # Go to Bind Pose must return to the new rest, not the old one.
            poses = set()
            for path in fn_paths:
                poses.update(cmds.listConnections(path + ".bindPose", source=False, destination=True,
                                                  type="dagPose") or [])
            for pose in poses:
                cmds.dagPose(fn_paths, reset=True, name=pose)
        bindinfo.remember(payload, mapping)
    return root


def _logical(skin, influence):
    from maya.api import OpenMayaAnim as oma
    fn = oma.MFnSkinCluster(build.node_object(skin))
    return fn.indexForInfluenceObject(om.MDagPath.getAPathTo(build.node_object(influence)))
