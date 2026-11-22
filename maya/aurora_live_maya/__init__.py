"""Aurora LiveMesh for Maya: preview rigged and static mesh edits live in
Aurora Viewer, without saving or uploading. Maya 2022 and later (Python 3).

Open the window with:

    import aurora_live_maya
    aurora_live_maya.show()
"""

VERSION = "0.5.0"


def show():
    from . import ui
    return ui.show()
