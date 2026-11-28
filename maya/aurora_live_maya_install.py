"""Install Aurora LiveMesh into the user's Maya scripts folder (run by
install.mel). Nothing outside the user's own Maya preferences is written."""

import os
import shutil
import sys

PACKAGE = "aurora_live_maya"
SHELF = "Aurora"
COMMAND = "import aurora_live_maya\naurora_live_maya.show()"


def install(folder):
    from maya import cmds, mel

    source = os.path.join(folder, PACKAGE)
    if not os.path.isfile(os.path.join(source, "__init__.py")):
        cmds.confirmDialog(title="Aurora LiveMesh", message="Extract the whole ZIP, then drag install.mel again.")
        return
    scripts = cmds.internalVar(userScriptDir=True)
    target = os.path.join(scripts, PACKAGE)
    if os.path.normcase(os.path.abspath(source)) != os.path.normcase(os.path.abspath(target)):
        _close_running()
        # Copied aside first: a failed copy keeps the installed plug-in.
        fresh = target + "_new"
        shutil.rmtree(fresh, ignore_errors=True)
        shutil.copytree(source, fresh, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        if os.path.isdir(target):
            shutil.rmtree(target)
        os.rename(fresh, target)
    if scripts not in sys.path:
        sys.path.append(scripts)
    # Import the installed copy, not the extracted one install.mel put first.
    while folder in sys.path:
        sys.path.remove(folder)
    for name in [name for name in sys.modules if name == PACKAGE or name.startswith(PACKAGE + ".")]:
        del sys.modules[name]
    _shelf_button(cmds, mel)
    import aurora_live_maya
    aurora_live_maya.show()
    cmds.inViewMessage(assistMessage="Aurora LiveMesh {} installed".format(aurora_live_maya.VERSION),
                       position="topCenter", fade=True)


def _close_running():
    """Stop a running session of an older copy before replacing its files."""
    module = sys.modules.get(PACKAGE + ".session")
    if module is not None and getattr(module, "_session", None) is not None:
        try:
            module._session.close()
        except Exception:
            pass
        module._session = None


def _shelf_button(cmds, mel):
    top = mel.eval("$aurora_tmp = $gShelfTopLevel")
    if not cmds.shelfLayout(SHELF, exists=True):
        mel.eval('addNewShelfTab "{}"'.format(SHELF))
    for button in cmds.shelfLayout(SHELF, query=True, childArray=True) or []:
        if cmds.shelfButton(button, exists=True) and cmds.shelfButton(button, query=True, label=True) == "LiveMesh":
            cmds.deleteUI(button)
    cmds.shelfButton(parent=top + "|" + SHELF, label="LiveMesh", imageOverlayLabel="Live", image="pythonFamily.png",
                     annotation="Aurora LiveMesh", sourceType="python", command=COMMAND)
