"""The Aurora LiveMesh window in Maya, built with maya.cmds."""

from maya import cmds

from . import convert, session


LOD_MODES = {"Existing or automatic": "fallback", "Existing mesh": "existing", "Automatic": "auto"}
LOD_BUDGETS = {"Percentage": "ratio", "Triangles": "triangles"}


def capture_up():
    return cmds.upAxis(query=True, axis=True).upper()


WINDOW = "auroraLiveMayaWindow"
# Maya stores every length in centimetres whatever its preferences show; a kit
# built "one unit per metre" in a centimetre scene has 1 cm per metre.
UNIT_LABELS = {"Automatic (from the rig)": "auto", "Real size": "cm", "1 Maya centimetre = 1 metre": "m"}


class Window:
    def __init__(self):
        self.live = session.get()
        if cmds.window(WINDOW, exists=True):
            cmds.deleteUI(WINDOW)
        from . import VERSION
        cmds.window(WINDOW, title="Aurora LiveMesh " + VERSION, widthHeight=(430, 640))
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
                                        doubleClickCommand=lambda *_: self.select_in_scene(),
                                        selectCommand=lambda *_: self.load_lods())
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
        cmds.rowLayout(numberOfColumns=2, adjustableColumn=2)
        cmds.text(label="Scene units ")
        self.units = cmds.optionMenu(changeCommand=lambda label: self.live.set_units(UNIT_LABELS[label]),
                                     annotation="How large the scene is. Automatic reads it from the rig's pelvis "
                                                "height; many Second Life kits are built at one unit per metre in a "
                                                "centimetre scene.")
        for label in UNIT_LABELS:
            cmds.menuItem(label=label)
        cmds.optionMenu(self.units, edit=True, select=list(UNIT_LABELS.values()).index(self.live.units()) + 1)
        cmds.setParent("..")
        self.unit_text = cmds.text(label="", align="left")
        self.lod_frame = cmds.frameLayout(label="Import / LOD", collapsable=True, collapse=True, marginWidth=4)
        self.lod_name = cmds.text(label="Select one mesh in the list.", align="left")
        self.lod_rows = {}
        for level, ratio in convert.LOD_LEVELS:
            cmds.rowLayout(numberOfColumns=5, adjustableColumn=3, columnWidth5=(52, 130, 80, 84, 60))
            cmds.text(label=level.title())
            mode = cmds.optionMenu(changeCommand=lambda *_: self.save_lods())
            for label in LOD_MODES:
                cmds.menuItem(label=label)
            name = cmds.textField(placeholderText="by name", changeCommand=lambda *_: self.save_lods(),
                                  annotation="A mesh for this level; empty: the High mesh's name with the suffix "
                                             "(_medium, _low, _lowest)")
            budget = cmds.optionMenu(changeCommand=lambda *_: self.save_lods())
            for label in LOD_BUDGETS:
                cmds.menuItem(label=label)
            # A starting value: Maya warns when a field starts below its minimum.
            value = cmds.floatField(minValue=0.1, maxValue=200000, value=ratio, precision=1,
                                    changeCommand=lambda *_: self.save_lods())
            cmds.setParent("..")
            self.lod_rows[level] = (mode, name, budget, value)
        cmds.text(label="Click Import in Aurora's LiveMesh list. Your meshes stay unchanged.", align="left",
                  wordWrap=True)
        cmds.setParent("..")
        cmds.text(label="Only you see this preview. Nothing is uploaded.", align="left", wordWrap=True)
        cmds.showWindow(WINDOW)
        self.live.listeners.append(self.refresh)
        cmds.scriptJob(uiDeleted=(WINDOW, self.closed))
        self._rows = None
        self.refresh()
        self.load_lods()

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

    def _lod_identity(self):
        selected = self._selected()
        return selected[0] if len(selected) == 1 else None

    def load_lods(self):
        identity = self._lod_identity()
        enabled = identity is not None
        node = self.live.node(identity) if enabled else None
        cmds.text(self.lod_name, edit=True,
                  label=(node.rsplit("|", 1)[-1] + " (High)") if node else "Select one mesh in the list.")
        settings = self.live.lod_settings(identity) if enabled else {}
        for level, (mode, name, budget, value) in self.lod_rows.items():
            spec = settings.get(level) or convert.lod_default(level)
            cmds.optionMenu(mode, edit=True, enable=enabled, select=list(LOD_MODES.values()).index(spec["mode"]) + 1)
            cmds.textField(name, edit=True, enable=enabled and spec["mode"] != "auto", text=spec["object"])
            cmds.optionMenu(budget, edit=True, enable=enabled and spec["mode"] != "existing",
                            select=list(LOD_BUDGETS.values()).index(spec["budget"]) + 1)
            cmds.floatField(value, edit=True, enable=enabled and spec["mode"] != "existing", value=spec["value"])

    def save_lods(self):
        identity = self._lod_identity()
        if identity is None:
            return
        levels = {}
        for level, (mode, name, budget, value) in self.lod_rows.items():
            spec = convert.lod_default(level)
            spec.update(mode=LOD_MODES[cmds.optionMenu(mode, query=True, value=True)],
                        object=cmds.textField(name, query=True, text=True).strip().rsplit("|", 1)[-1],
                        budget=LOD_BUDGETS[cmds.optionMenu(budget, query=True, value=True)],
                        value=max(0.1, cmds.floatField(value, query=True, value=True)))
            levels[level] = spec
        self.live.set_lod_settings(identity, levels)
        self.load_lods()

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
        unit = self.live.unit
        text = "" if unit is None else "1 Maya cm = {:g} m, {}-up".format(unit * 100, capture_up())
        if cmds.text(self.unit_text, query=True, label=True) != text:
            cmds.text(self.unit_text, edit=True, label=text)
        message = self.live.message
        if cmds.text(self.message, query=True, label=True) != message:
            cmds.text(self.message, edit=True, label=message)


_window = None


def show():
    global _window
    _window = Window()
    return _window
