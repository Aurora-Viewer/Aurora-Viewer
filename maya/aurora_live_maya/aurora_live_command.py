"""A Maya command that makes the plug-in's OpenMaya edits undoable.

OpenMaya calls such as MFnSkinCluster.setWeights and MFnMesh.setPoints are
fast but are not recorded in Maya's undo queue. The plug-in loads this file
as a Maya plug-in and runs such an edit through `auroraLiveEdit`: the edit is
handed over in aurora_live_maya.build.PENDING as a function that applies it
and returns the function that reverts it. Maya then keeps it in the undo
queue like any command (undo and redo included).
"""

import sys

import maya.api.OpenMaya as om

NAME = "auroraLiveEdit"


def maya_useNewAPI():
    """Tells Maya this plug-in uses the Python API 2.0."""


class Edit(om.MPxCommand):
    def __init__(self):
        om.MPxCommand.__init__(self)
        self._apply = None
        self._revert = None

    @staticmethod
    def creator():
        return Edit()

    def isUndoable(self):
        return True

    def doIt(self, _args):
        build = sys.modules.get("aurora_live_maya.build")
        self._apply = getattr(build, "PENDING", None) if build else None
        if build is not None:
            build.PENDING = None
        if self._apply is None:
            raise RuntimeError("auroraLiveEdit runs edits of the Aurora LiveMesh plug-in only")
        self.redoIt()

    def redoIt(self):
        self._revert = self._apply()

    def undoIt(self):
        if self._revert is not None:
            self._revert()


def initializePlugin(plugin):
    om.MFnPlugin(plugin, "Aurora Viewer contributors", "1.0").registerCommand(NAME, Edit.creator)


def uninitializePlugin(plugin):
    om.MFnPlugin(plugin).deregisterCommand(NAME)
