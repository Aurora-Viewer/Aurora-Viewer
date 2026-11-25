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

from . import bindinfo, build, controls, convert, rigs, shape_rules


def _previous(joint, payload_joint):
    if cmds.attributeQuery("auroraShapeBind", node=joint, exists=True):
        values = cmds.getAttr(joint + ".auroraShapeBind") or []
        if len(values) == 16:
            return values
    return payload_joint["reference_bind"]


def _orig(skin, mesh):
    """The rest shape feeding a skin. Maya's own plumbing (groupParts, tweak,
    groupId) may sit between them; any other deformer before the skin (a
    blend shape, a wrap...) is refused, as its shapes would need moving too."""
    for node in cmds.listHistory(skin, pruneDagObjects=True) or []:
        if node == skin or cmds.nodeType(node) in ("skinCluster", "tweak", "groupParts", "groupId"):
            continue
        if "geometryFilter" in (cmds.nodeType(node, inherited=True) or []):
            raise ValueError("{}: a {} comes before its skin; apply the shape with MayaStar or the Blender "
                             "plug-in".format(mesh.rsplit("|", 1)[-1], cmds.nodeType(node)))
    found = cmds.deformableShape(mesh, originalGeometry=True) or []
    orig = found[0].split(".")[0] if found and found[0] else ""
    if not orig or not cmds.objExists(orig):
        raise ValueError("{}: no rest shape found for its skin".format(mesh.rsplit("|", 1)[-1]))
    return cmds.ls(orig, long=True)[0]


def _skins(root):
    """Every skinCluster using one of this rig's joints; influences outside
    the rig are left in place."""
    joints = set([root] + (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []))
    result = []
    for skin in cmds.ls(type="skinCluster") or []:
        influences = cmds.ls(cmds.skinCluster(skin, query=True, influence=True) or [], long=True)
        if set(influences) & joints:
            result.append(skin)
    return result


def _driven(mapping):
    """Refuse joints that controls or animation drive: moving their rest
    under a constraint would fight it."""
    for name, joint in mapping.items():
        for attribute in controls.DRIVEN:
            if cmds.listConnections(joint + "." + attribute, source=True, destination=False, skipConversionNodes=True):
                raise ValueError("{} is driven by a control, constraint or animation; remove the Aurora "
                                 "controls (or that driver) before applying a shape".format(name))


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


def rig_unit(mapping, up_axis):
    """Metres per Maya unit of this rig, from its pelvis height; real
    centimetres when it cannot tell."""
    heads = {name: build.world_position(path) for name, path in mapping.items()}
    return convert.rig_unit(heads, up_axis) or convert.CM


def apply(payload, root, up_axis, unit=None):
    """Reshape a plain SL rig and its skinned meshes; returns the rig.
    `unit` (metres per Maya unit) None reads it from the rig itself."""
    shape_rules.validate(payload, shape_rules.skeleton())
    mapping = rigs.joint_map(root)
    if not set(rigs.CORE).issubset(mapping):
        raise ValueError("The selected skeleton is not a Second Life rig")
    _driven(mapping)
    build.check_rest(root)
    unit = unit or rig_unit(mapping, up_axis)
    heads = {name: build.world_position(path) for name, path in mapping.items()}
    base = convert.base_frame(up_axis, unit)
    frame = convert.multiply(convert.facing({name: convert.point(base, head) for name, head in heads.items()}), base)
    moves = deltas(payload, mapping, frame)
    # A joint the shape does not name follows its nearest named ancestor.
    by_joint = {}
    for name, path in mapping.items():
        current, matrix = path, None
        while current and matrix is None:
            matrix = moves.get(rigs.sl_name(current))
            parent = cmds.listRelatives(current, parent=True, fullPath=True, type="joint")
            current = parent[0] if parent else None
        by_joint[path] = matrix or convert.identity()
    plans = []
    for skin in _skins(root):
        meshes = cmds.skinCluster(skin, query=True, geometry=True) or []
        if not meshes:
            continue  # deforms nothing
        if len(meshes) > 1:
            raise ValueError("{} skins several meshes; give each its own skin first".format(skin))
        mesh = meshes[0]
        orig = _orig(skin, mesh)
        geometry = convert.from_maya(cmds.getAttr(skin + ".geomMatrix"))
        weights = build.get_weights(skin, mesh)
        points = om.MFnMesh(build._shape_path(orig)).getPoints(om.MSpace.kObject)
        if len(points) != len(weights):
            raise ValueError("{}: its rest mesh and its weights disagree".format(mesh.rsplit("|", 1)[-1]))
        geometry_inverse = convert.inverse(geometry)
        moved = [convert.point(geometry_inverse, convert.point(_blend(groups, by_joint),
                                                                convert.point(geometry, (p.x, p.y, p.z))))
                 for p, groups in zip(points, weights)]
        plans.append((skin, orig, moved))
    ordered = sorted(mapping.items(), key=lambda item: item[1].count("|"))
    joints = [path for _name, path in ordered]
    with build.undoable("Aurora apply shape"):
        # Parents first: a child keeps its own new head after its parent moved.
        for name, path in ordered:
            cmds.xform(path, worldSpace=True,
                       translation=[build.to_ui(v) for v in convert.point(by_joint[path], heads[name])])
        rig_joints = set(joints)
        for skin, orig, moved in plans:
            build.set_points(orig, moved)
            # The skin binds at the rig joints' new place.
            for path in build.influences(skin):
                if path in rig_joints:
                    cmds.setAttr("{}.bindPreMatrix[{}]".format(skin, _logical(skin, path)),
                                 cmds.getAttr(path + ".worldInverseMatrix[0]"), type="matrix")
        # Go to Bind Pose must return to the new rest: reset every rig joint
        # its poses hold, not only the skins' influences.
        poses = set()
        for path in joints:
            poses.update(cmds.listConnections(path + ".bindPose", source=False, destination=True,
                                              type="dagPose") or [])
        for pose in poses:
            members = [path for path in joints if pose in (cmds.listConnections(
                path + ".bindPose", source=False, destination=True, type="dagPose") or [])]
            if members:
                cmds.dagPose(members, reset=True, name=pose)
        bindinfo.remember(payload, mapping)
    return root


def _logical(skin, influence):
    from maya.api import OpenMayaAnim as oma
    fn = oma.MFnSkinCluster(build.node_object(skin))
    return fn.indexForInfluenceObject(om.MDagPath.getAPathTo(build.node_object(influence)))
