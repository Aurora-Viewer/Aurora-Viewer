"""Apply a shape from Aurora to a MayaStar rig through MayaStar's own import.

Like the Blender plug-in with Avastar (blender/aurora_live/shape_apply.py),
the rig is never edited by hand: MayaStar's "Import XML shape"
(Mayastar_xmlShape.ShapeImport.importShape) reads the Legacy Viewer's
appearance XML that Aurora sends with the shape and drives MayaStar's own
sliders, controls and meshes. That import asks for its file with
cmds.fileDialog2; for the length of the call only, its module sees a cmds
whose fileDialog2 answers with Aurora's file. Maya itself and MayaStar's files
are not changed, and no MayaStar code is bundled.

Afterwards each SL joint of the rig keeps the shape's bind, as the Blender
plug-in does after Avastar, so a garment previews with the shape once.
"""

import importlib
import os
import sys
import tempfile

from maya import cmds

from . import bindinfo, convert

MODULE = "Mayastar" + "_xmlShape"
# The control MayaStar's import reads first: a MayaStar rig is in the scene.
CONTROL = "Female_Male"


def importer():
    """MayaStar's ShapeImport class, or None when MayaStar is not loaded."""
    module = sys.modules.get(MODULE)
    if module is None:
        try:
            module = importlib.import_module(MODULE)
        except ImportError:
            return None, None
    shape_import = getattr(module, "ShapeImport", None)
    if not callable(getattr(shape_import, "importShape", None)) or not hasattr(module, "cmds"):
        return module, None
    return module, shape_import


class _Answer:
    """maya.cmds for MayaStar's import, with its file dialog answered."""

    def __init__(self, path):
        self._path = path

    def fileDialog2(self, *args, **kwargs):
        return [self._path]

    def __getattr__(self, name):
        return getattr(cmds, name)


def rig_joints(names):
    """SL name -> joint of the rig to reshape: the selected rig, else the only
    rig of the scene outside a namespace (received shapes have their own)."""
    selected = cmds.ls(selection=True, long=True) or []
    roots = {"|" + node.split("|")[1] for node in selected if node.startswith("|")}
    candidates = []
    for root in sorted(roots) or [None]:
        joints = (cmds.listRelatives(root, allDescendents=True, type="joint", fullPath=True) or []) if root else (
            cmds.ls(type="joint", long=True) or [])
        if root and cmds.nodeType(root) == "joint":
            joints.append(root)
        mapping = {}
        for joint in joints:
            short = joint.rsplit("|", 1)[-1]
            if root is None and ":" in short:
                continue
            override = cmds.getAttr(joint + ".auroraJoint") if cmds.attributeQuery(
                "auroraJoint", node=joint, exists=True) else None
            name = convert.joint_name(short, override)
            if name in names:
                if name in mapping:
                    raise ValueError("Several rigs or joints are named " + name + "; select the MayaStar rig")
                mapping[name] = joint
        if {"mPelvis", "mTorso", "mChest", "mHead"}.issubset(mapping):
            candidates.append(mapping)
    if len(candidates) != 1:
        raise ValueError("Select the MayaStar rig in Maya first")
    return candidates[0]


def apply(payload):
    """Reshape the MayaStar rig with the shape; one undo step, undone if it fails."""
    module, shape_import = importer()
    if shape_import is None:
        raise ValueError("Applying a shape in Maya uses MayaStar's shape import: open MayaStar first")
    if not cmds.objExists(CONTROL):
        raise ValueError("No MayaStar rig in this scene")
    archetype = payload.get("archetype")
    if not isinstance(archetype, str) or "<linden_genepool" not in archetype:
        raise ValueError("Update Aurora Viewer to apply a shape with MayaStar")
    joints = rig_joints({joint["name"] for joint in payload["joints"]})
    handle, path = tempfile.mkstemp(prefix="aurora-shape-", suffix=".xml")
    original, opened, done = module.cmds, False, False
    undo = cmds.undoInfo(query=True, state=True)
    try:
        # MayaStar opens the file with the system's default encoding: ASCII
        # with character references reads the same everywhere.
        with os.fdopen(handle, "wb") as file:
            file.write(archetype.encode("ascii", "xmlcharrefreplace"))
        cmds.undoInfo(openChunk=True, chunkName="Aurora shape (MayaStar)")
        opened = True
        # Never an empty step: undoing a failure must not reach the user's own
        # previous action (build.undoable).
        cmds.delete(cmds.createNode("network", name="auroraLiveStep", skipSelect=True))
        try:
            module.cmds = _Answer(path)
            shape_import.importShape()
        finally:
            module.cmds = original
        bindinfo.remember(payload, joints)
        done = True
    finally:
        module.cmds = original
        if opened:
            cmds.undoInfo(closeChunk=True)
            if undo and not done:
                try:
                    cmds.undo()
                except RuntimeError:
                    pass
        try:
            os.remove(path)
        except OSError:
            pass
    return joints["mPelvis"].split("|")[1]
