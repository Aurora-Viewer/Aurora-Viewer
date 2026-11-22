"""Weight tools for SL rigs in Maya (Blender plug-in: Automatic weight
painting, Transfer and normalize weights), on Maya's own skinning:

- Automatic weights binds the selected meshes to the rig's SL joints with
  Maya's heat-map bind, the classic body only or every deforming joint but
  collision volumes and the folded Bento spine, then keeps the strongest four.
- Transfer weights copies a skinned body's weights to the selected meshes
  with copySkinWeights (closest point on its surface, or closest vertex),
  refused as a whole when a vertex is farther than the maximum distance.
- Normalize to 4 keeps each vertex's strongest four influences, normalized.

Each run is one undo step and is cancelled as a whole on any failure. The rig
must stand in its bind pose: a bind in another pose would bake it in.
"""

import maya.api.OpenMaya as om
from maya import cmds

from . import build, convert, rig_rules, rigs


def _check_rest(root):
    """Refuse a rig posed away from its bind pose."""
    poses = set()
    for joint in [root] + (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []):
        poses.update(cmds.listConnections(joint + ".bindPose", source=False, destination=True, type="dagPose") or [])
    for pose in poses:
        off = cmds.dagPose(pose, query=True, atPose=True) or []
        if off:
            raise ValueError("Put the rig in its bind pose first (Skin > Go to Bind Pose); "
                             "{} joints are posed".format(len(off)))


def _unbind(mesh):
    skin = build.skin_of(mesh)
    if skin:
        cmds.skinCluster(skin, edit=True, unbind=True)


def limit(skin, mesh, count=4):
    """Keep each vertex's strongest `count` influences, normalized; a vertex
    left without weight takes its nearest weighted neighbour's."""
    weights = build.get_weights(skin, mesh)
    names = sorted({joint for groups in weights for joint, _weight in groups})
    index = {name: number for number, name in enumerate(names)}
    joints, values = [], []
    for groups in weights:
        ranked = convert.rank([(index[joint], weight) for joint, weight in groups])
        joints.append(ranked[0])
        values.append(ranked[1])
    shape = build._shape_path(mesh)
    points = om.MFnMesh(shape).getPoints(om.MSpace.kWorld)
    positions = [(p.x, p.y, p.z) for p in points]
    convert.complete(joints, values, positions, list(range(len(positions))))
    result = [[(names[joint], weight) for joint, weight in zip(j, w) if weight > 0] for j, w in zip(joints, values)]
    cmds.setAttr(skin + ".maxInfluences", count)
    cmds.setAttr(skin + ".maintainMaxInfluences", True)
    build.set_weights(skin, mesh, result)


def normalize(meshes):
    """Normalize to 4 weights, on every selected skinned mesh."""
    skins = [(mesh, build.skin_of(mesh)) for mesh in meshes]
    if not skins or any(skin is None for _mesh, skin in skins):
        raise ValueError("Select skinned meshes")
    with build.undoable("Aurora normalize weights"):
        for mesh, skin in skins:
            limit(skin, mesh)
    return [mesh for mesh, _skin in skins]


def automatic(meshes, root, mode="body"):
    """Replace the meshes' weights with Maya's heat-map bind to the rig."""
    if not meshes:
        raise ValueError("Select the meshes to weight")
    mapping = rigs.joint_map(root)
    allowed = rig_rules.weight_joints(rig_rules.default_joints(), mode)
    joints = [mapping[name] for name in allowed if name in mapping]
    if len(joints) < 4:
        raise ValueError("This rig has too few SL joints for automatic weights")
    _check_rest(root)
    with build.undoable("Aurora automatic weights"):
        for mesh in meshes:
            _unbind(mesh)
            skin = cmds.skinCluster(joints, mesh, toSelectedBones=True, bindMethod=2, heatmapFalloff=0.68,
                                    maximumInfluences=4, obeyMaxInfluences=True, normalizeWeights=1,
                                    removeUnusedInfluence=False)[0]
            weights = build.get_weights(skin, mesh)
            missing = sum(1 for groups in weights if not groups)
            if missing:
                raise ValueError("{}: {} vertices got no weight; check the topology or paint them".format(
                    mesh.rsplit("|", 1)[-1], missing))
            limit(skin, mesh)
    return meshes


def _distances(source, mesh):
    """Each vertex's world distance to the source's surface."""
    surface = om.MFnMesh(build._shape_path(source))
    points = om.MFnMesh(build._shape_path(mesh)).getPoints(om.MSpace.kWorld)
    return [surface.getClosestPoint(point, om.MSpace.kWorld)[0].distanceTo(point) for point in points]


def transfer(source, targets, nearest_vertex=False, max_distance=0.0):
    """Copy a skinned source's weights to the targets, which are (re)bound to
    the source's joints; `max_distance` in Maya units, 0 for unlimited."""
    source_skin = build.skin_of(source)
    if not source_skin:
        raise ValueError("The weight source must be a skinned mesh")
    targets = [target for target in targets if target != source]
    if not targets:
        raise ValueError("Select the meshes to receive the weights")
    joints = cmds.skinCluster(source_skin, query=True, influence=True) or []
    root = rigs.top_joint(joints[0]) if joints else None
    if root:
        _check_rest(root)
    if max_distance > 0:
        limit_distance = build.from_ui(max_distance)  # the field is in the preferences' unit
        for target in targets:
            far = max(_distances(source, target))
            if far > limit_distance:
                raise ValueError("{} has a vertex {:.3g} from the source, more than the maximum {:g}".format(
                    target.rsplit("|", 1)[-1], build.to_ui(far), max_distance))
    with build.undoable("Aurora transfer weights"):
        for target in targets:
            _unbind(target)
            skin = cmds.skinCluster(joints, target, toSelectedBones=True, bindMethod=0, maximumInfluences=4,
                                    obeyMaxInfluences=False, normalizeWeights=1)[0]
            cmds.copySkinWeights(sourceSkin=source_skin, destinationSkin=skin, noMirror=True,
                                 surfaceAssociation="closestComponent" if nearest_vertex else "closestPoint",
                                 influenceAssociation=["oneToOne", "name"], normalize=True)
            limit(skin, target)
    return targets
