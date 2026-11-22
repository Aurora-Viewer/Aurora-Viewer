"""Find Second Life rigs in the scene: a hierarchy of joints named after SL
joints (namespaces ignored, or an `auroraJoint` string attribute)."""

from maya import cmds

from . import capture, convert

CORE = ("mPelvis", "mTorso", "mChest", "mHead")


def sl_name(joint):
    override = (cmds.getAttr(joint + ".auroraJoint")
                if cmds.attributeQuery("auroraJoint", node=joint, exists=True) else None)
    return convert.joint_name(joint.rsplit("|", 1)[-1], override)


def top_joint(node):
    """The highest joint above (or at) a node."""
    path = cmds.ls(node, long=True)[0]
    parts = path.split("|")
    for index in range(2, len(parts) + 1):
        candidate = "|".join(parts[:index])
        if cmds.nodeType(candidate) == "joint":
            return candidate
    return None


def joint_map(root):
    """SL name -> joint path, for the joints at and under `root`."""
    joints = [root] + (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or [])
    mapping = {}
    for joint in joints:
        name = sl_name(joint)
        if name in mapping:
            raise ValueError("Two joints of this rig map to the SL joint " + name)
        mapping[name] = joint
    return mapping


def is_rig(root):
    try:
        return set(CORE).issubset(joint_map(root))
    except ValueError:
        return False


def rigs():
    """Top joints of every SL rig in the scene."""
    tops = {top_joint(joint) for joint in cmds.ls(type="joint", long=True) or []}
    return sorted(top for top in tops if top and is_rig(top))


def rig_of_mesh(mesh):
    """The rig a mesh is skinned to, or None."""
    shape = capture.shape_of(mesh) if cmds.nodeType(mesh) == "transform" else mesh
    skin = capture.skin_of(shape) if shape else None
    if not skin:
        return None
    for influence in cmds.skinCluster(skin, query=True, influence=True) or []:
        top = top_joint(influence)
        if top and is_rig(top):
            return top
    return None


def resolve(selection=None):
    """The rig to work on: a selected joint's rig, a selected mesh's rig, or
    the scene's only rig. Raises when that is ambiguous."""
    selection = cmds.ls(selection=True, long=True) if selection is None else selection
    found = set()
    for node in selection or []:
        if cmds.nodeType(node) == "joint":
            top = top_joint(node)
            if top and is_rig(top):
                found.add(top)
        else:
            rig = rig_of_mesh(node)
            if rig:
                found.add(rig)
    if len(found) == 1:
        return found.pop()
    if found:
        raise ValueError("The selection uses several rigs; select one")
    every = rigs()
    if len(every) == 1:
        return every[0]
    raise ValueError("Select the rig (or a mesh skinned to it)" if every else "No Second Life rig in this scene")


def meshes(selection=None):
    """Selected mesh transforms."""
    selection = cmds.ls(selection=True, long=True) if selection is None else selection
    result = []
    for node in selection or []:
        if cmds.nodeType(node) == "mesh":
            node = cmds.listRelatives(node, parent=True, fullPath=True)[0]
        if cmds.nodeType(node) == "transform" and capture.shape_of(node) and node not in result:
            result.append(node)
    return result
