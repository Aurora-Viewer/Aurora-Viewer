"""The Aurora LiveMesh window in Maya, built with maya.cmds."""

from maya import cmds

from . import session

WINDOW = "auroraLiveMayaWindow"


class Window:
    def __init__(self):
        self.live = session.get()
        if cmds.window(WINDOW, exists=True):
            cmds.deleteUI(WINDOW)
        from . import VERSION
        cmds.window(WINDOW, title="Aurora LiveMesh " + VERSION, widthHeight=(380, 460))
        cmds.columnLayout(adjustableColumn=True, rowSpacing=6, columnAttach=("both", 8))
        cmds.text(label="Start the preview in Aurora's LiveMesh window, then connect.", align="left", wordWrap=True)
        cmds.rowLayout(numberOfColumns=3, adjustableColumn=3, columnWidth3=(40, 80, 200))
        cmds.text(label="Port")
        self.port = cmds.intField(value=session.option(session.PORT, session.DEFAULT_PORT), minValue=1024, maxValue=65535)
        self.connect = cmds.button(label="Connect", command=lambda *_: self.toggle())
        cmds.setParent("..")
        self.status = cmds.text(label="", align="left", wordWrap=True)
        cmds.separator(style="none", height=4)
        cmds.text(label="Synchronized meshes", align="left", font="boldLabelFont")
        self.list = cmds.textScrollList(allowMultiSelection=True, height=170,
                                        doubleClickCommand=lambda *_: self.select_in_scene())
        cmds.rowLayout(numberOfColumns=3, adjustableColumn=3)
        cmds.button(label="Add selected", command=lambda *_: self.add())
        cmds.button(label="Remove", command=lambda *_: self.remove())
        cmds.button(label="Select in scene", command=lambda *_: self.select_in_scene())
        cmds.setParent("..")
        self.message = cmds.text(label="", align="left", wordWrap=True)
        cmds.separator(style="none", height=4)
        cmds.rowLayout(numberOfColumns=3, adjustableColumn=3)
        self.auto = cmds.checkBox(label="Automatic updates", value=bool(session.option(session.AUTO, 1)),
                                  changeCommand=lambda value: session.set_option(session.AUTO, bool(value)))
        cmds.text(label="  every (s)")
        self.interval = cmds.floatField(value=session.option(session.INTERVAL, session.DEFAULT_INTERVAL), minValue=0.1,
                                        maxValue=3600.0, precision=2,
                                        changeCommand=lambda value: session.set_option(session.INTERVAL, float(value)))
        cmds.setParent("..")
        cmds.button(label="Update now", command=lambda *_: self.live.update_now())
        self.axes = cmds.checkBox(
            label="Use Second Life joint axes (ignore joint orientation)", value=self.live.bind_axes(),
            annotation="On: only each joint's bind position is used, with Second Life's own joint axes. "
                       "Off: the skin's bind matrices are used whole, as a Collada export writes them.",
            changeCommand=lambda value: self.live.set_bind_axes(bool(value)))
        cmds.text(label="Only you see this preview. Nothing is uploaded.", align="left", wordWrap=True)
        cmds.showWindow(WINDOW)
        self.live.listeners.append(self.refresh)
        cmds.scriptJob(uiDeleted=(WINDOW, self.closed))
        self._rows = None
        self.refresh()

    def toggle(self):
        if self.live.client is None:
            self.live.connect(cmds.intField(self.port, query=True, value=True))
        else:
            self.live.disconnect()
        self.refresh()

    def add(self):
        selected = cmds.ls(selection=True, long=True, type="transform") or []
        # A selected shape adds its transform.
        for shape in cmds.ls(selection=True, long=True, type="mesh") or []:
            selected += cmds.listRelatives(shape, parent=True, fullPath=True) or []
        if not self.live.add(selected):
            self.live.message = self.live.message or "Select mesh objects to add"
        self.refresh()

    def _selected(self):
        indices = cmds.textScrollList(self.list, query=True, selectIndexedItem=True) or []
        rows = self.live.rows()
        return [rows[index - 1][0] for index in indices if 0 < index <= len(rows)]

    def remove(self):
        self.live.remove(self._selected())
        self.refresh()

    def select_in_scene(self):
        nodes = [node for node in (self.live.node(identity) for identity in self._selected()) if node]
        if nodes:
            cmds.select(nodes, replace=True)

    def closed(self):
        if self.refresh in self.live.listeners:
            self.live.listeners.remove(self.refresh)

    def refresh(self):
        if not cmds.window(WINDOW, exists=True):
            self.closed()
            return
        connected = self.live.client is not None
        cmds.button(self.connect, edit=True, label="Disconnect" if connected else "Connect")
        cmds.intField(self.port, edit=True, enable=not connected)
        status = self.live.status()
        if cmds.text(self.status, query=True, label=True) != status:
            cmds.text(self.status, edit=True, label=status)
        rows = [name + ("  —  " + message if message else "") for _identity, name, message in self.live.rows()]
        if rows != self._rows:
            selected = cmds.textScrollList(self.list, query=True, selectIndexedItem=True) or []
            cmds.textScrollList(self.list, edit=True, removeAll=True)
            for row in rows:
                cmds.textScrollList(self.list, edit=True, append=row)
            for index in selected:
                if index <= len(rows):
                    cmds.textScrollList(self.list, edit=True, selectIndexedItem=index)
            self._rows = rows
        message = self.live.message
        if cmds.text(self.message, query=True, label=True) != message:
            cmds.text(self.message, edit=True, label=message)


_window = None


def show():
    global _window
    _window = Window()
    return _window
