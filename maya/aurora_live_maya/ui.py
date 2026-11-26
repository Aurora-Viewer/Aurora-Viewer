"""The Aurora LiveMesh window in Maya, built with maya.cmds, in tabs: Live
(connection and synchronized meshes), Import / LOD, Rig, Weights and
Controls."""

from maya import cmds

from . import convert, rig_rules, session

WINDOW = "auroraLiveMayaWindow"
LOD_MODES = {"Existing or automatic": "fallback", "Existing mesh": "existing", "Automatic": "auto"}
LOD_BUDGETS = {"Percentage": "ratio", "Triangles": "triangles"}
# Maya stores every length in centimetres whatever its preferences show; a kit
# built "one unit per metre" in a centimetre scene has 1 cm per metre.
UNIT_LABELS = {"Automatic (from the rig)": "auto", "Real size": "cm", "1 Maya centimetre = 1 metre": "m"}
WEIGHT_MODES = {"SL body": "body", "All deform joints": "all"}
LIMB_LABELS = {"HandLeft": "Left hand", "HandRight": "Right hand", "FootLeft": "Left foot", "FootRight": "Right foot"}


def capture_up():
    return cmds.upAxis(query=True, axis=True).upper()


def _row(count):
    return cmds.rowLayout(numberOfColumns=count, adjustableColumn=count)


class Window:
    def __init__(self):
        self.live = session.get()
        if cmds.window(WINDOW, exists=True):
            cmds.deleteUI(WINDOW)
        from . import VERSION
        cmds.window(WINDOW, title="Aurora LiveMesh " + VERSION, widthHeight=(440, 660))
        form = cmds.formLayout()
        tabs = cmds.tabLayout(innerMarginWidth=6, innerMarginHeight=6)
        cmds.formLayout(form, edit=True, attachForm=[(tabs, side, 0) for side in ("top", "left", "right", "bottom")])
        self._live_tab(tabs)
        self._lod_tab(tabs)
        self._rig_tab(tabs)
        self._weights_tab(tabs)
        self._controls_tab(tabs)
        cmds.showWindow(WINDOW)
        self.live.listeners.append(self.refresh)
        cmds.scriptJob(uiDeleted=(WINDOW, self.closed))
        # The tool tabs follow the selection (which rig), not a timer.
        cmds.scriptJob(event=("SelectionChanged", lambda: self.refresh(tools=True)), parent=WINDOW)
        self._rows = None
        self._entries = None
        self._layers = None
        self.refresh(tools=True)
        self.load_lods()

    # ------------------------------------------------------------------ tabs

    def _tab(self, tabs, label):
        layout = cmds.columnLayout(adjustableColumn=True, rowSpacing=6, columnAttach=("both", 6))
        cmds.tabLayout(tabs, edit=True, tabLabel=(layout, label))
        return layout

    def _live_tab(self, tabs):
        self._tab(tabs, "Live")
        cmds.text(label="Start the preview in Aurora's LiveMesh window, then connect.", align="left", wordWrap=True)
        cmds.rowLayout(numberOfColumns=3, adjustableColumn=3, columnWidth3=(40, 80, 200))
        cmds.text(label="Port")
        self.port = cmds.intField(value=session.option(session.PORT, session.DEFAULT_PORT), minValue=1024, maxValue=65535)
        self.connect = cmds.button(label="Connect", command=lambda *_: self.toggle())
        cmds.setParent("..")
        self.status = cmds.text(label="", align="left", wordWrap=True)
        cmds.text(label="Synchronized meshes", align="left", font="boldLabelFont")
        self.list = cmds.textScrollList(allowMultiSelection=True, height=170,
                                        doubleClickCommand=lambda *_: self.select_in_scene(),
                                        selectCommand=lambda *_: self.load_lods())
        _row(3)
        cmds.button(label="Add selected", command=lambda *_: self.add())
        cmds.button(label="Remove", command=lambda *_: self.run(lambda: self.live.remove(self._selected())))
        cmds.button(label="Select in scene", command=lambda *_: self.select_in_scene())
        cmds.setParent("..")
        _row(5)
        cmds.button(label="Pause", annotation="Keep the entry, take it out of the preview",
                    command=lambda *_: self.run(lambda: self.live.set_paused(self._selected(), True)))
        cmds.button(label="Resume", command=lambda *_: self.run(lambda: self.live.set_paused(self._selected(), False)))
        cmds.button(label="Group", annotation="One linkset of the selected meshes without a skin; the first is its root",
                    command=lambda *_: self.run(lambda: self.live.group(self._selected())))
        cmds.button(label="Ungroup", annotation="A root frees its linkset; a child leaves alone",
                    command=lambda *_: self.run(lambda: self.live.ungroup(self._selected())))
        cmds.button(label="Make root", command=lambda *_: self.run(lambda: self.live.make_root(self._one())))
        cmds.setParent("..")
        self.message = cmds.text(label="", align="left", wordWrap=True)
        cmds.rowLayout(numberOfColumns=3, adjustableColumn=3)
        cmds.checkBox(label="Automatic updates", value=bool(session.option(session.AUTO, 1)),
                      changeCommand=lambda value: session.set_option(session.AUTO, bool(value)))
        cmds.text(label="  every (s)")
        cmds.floatField(value=session.option(session.INTERVAL, session.DEFAULT_INTERVAL), minValue=0.1,
                        maxValue=3600.0, precision=2,
                        changeCommand=lambda value: session.set_option(session.INTERVAL, float(value)))
        cmds.setParent("..")
        cmds.button(label="Update now", command=lambda *_: self.live.update_now())
        cmds.checkBox(label="Use Second Life joint axes (ignore joint orientation)", value=self.live.bind_axes(),
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
        cmds.text(label="Only you see this preview. Nothing is uploaded.", align="left", wordWrap=True)
        cmds.setParent("..")

    def _lod_tab(self, tabs):
        self._tab(tabs, "Import / LOD")
        self.lod_name = cmds.text(label="Select one mesh in the Live list.", align="left")
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

    def _rig_tab(self, tabs):
        self._tab(tabs, "Rig")
        cmds.text(label="Create an SL avatar rig", align="left", font="boldLabelFont")
        self.with_shirt = cmds.checkBox(label="Add a rigged test T-shirt", value=True)
        cmds.button(label="Create SL avatar rig", annotation="All 133 Bento bones and 26 Fitted Mesh volumes at the "
                    "standard SL rest; never replaces anything", command=lambda *_: self.run(self.create_rig))
        cmds.text(label="Skeleton display", align="left", font="boldLabelFont")
        cmds.button(label="Sort the selected rig into SL layers",
                    command=lambda *_: self.run(self.sort_layers))
        _row(2)
        cmds.button(label="Show all", command=lambda *_: self.run(lambda: self.all_layers(True)))
        cmds.button(label="Hide all", command=lambda *_: self.run(lambda: self.all_layers(False)))
        cmds.setParent("..")
        self.layer_box = cmds.columnLayout(adjustableColumn=True)
        cmds.setParent("..")
        cmds.text(label="Hiding a part changes the display only, never deformation.", align="left", wordWrap=True)
        cmds.setParent("..")

    def _weights_tab(self, tabs):
        self._tab(tabs, "Weights")
        cmds.text(label="Uses the selected rig, the rig of the selected meshes, or the scene's only rig. "
                        "The rig must be in its bind pose. Each tool is one undo step.", align="left", wordWrap=True)
        cmds.text(label="Automatic weights", align="left", font="boldLabelFont")
        _row(2)
        self.weight_mode = cmds.optionMenu()
        for label in WEIGHT_MODES:
            cmds.menuItem(label=label)
        cmds.button(label="Recalculate weights", annotation="Replace the selected meshes' weights with Maya's heat-map "
                    "bind, then keep the strongest four", command=lambda *_: self.run(self.automatic))
        cmds.setParent("..")
        cmds.text(label="Weight transfer", align="left", font="boldLabelFont")
        _row(2)
        self.source = cmds.textField(placeholderText="weight source mesh")
        cmds.button(label="Use selected", command=lambda *_: self.pick_source())
        cmds.setParent("..")
        _row(4)
        self.nearest = cmds.optionMenu()
        cmds.menuItem(label="Nearest surface")
        cmds.menuItem(label="Nearest vertex")
        cmds.text(label=" Max distance")
        self.max_distance = cmds.floatField(value=0.0, minValue=0.0, precision=3,
                                            annotation="0 for unlimited, in the preferences' unit")
        cmds.button(label="Transfer weights", command=lambda *_: self.run(self.transfer))
        cmds.setParent("..")
        cmds.text(label="Weight cleanup", align="left", font="boldLabelFont")
        cmds.button(label="Normalize to 4 weights", command=lambda *_: self.run(self.normalize))
        cmds.setParent("..")

    def _controls_tab(self, tabs):
        self._tab(tabs, "Controls")
        cmds.text(label="Pose the selected rig in Maya. Aurora still plays its own animations on the rest mesh.",
                  align="left", wordWrap=True)
        _row(2)
        cmds.button(label="Add root / COG", command=lambda *_: self.run(self.root_cog))
        cmds.button(label="Create IK controls", command=lambda *_: self.run(self.ik))
        cmds.setParent("..")
        self.limbs = {}
        _row(4)
        for limb, label in LIMB_LABELS.items():
            self.limbs[limb] = cmds.checkBox(label=label, value=False, enable=False,
                                             changeCommand=lambda value, limb=limb: self.run(
                                                 lambda: self.set_ik(limb, bool(value))))
        cmds.setParent("..")
        cmds.text(label="Look at", align="left", font="boldLabelFont")
        self.focus = {}
        for which in ("Classic", "Bento"):
            _row(4)
            cmds.button(label=which + " eyes +", command=lambda *_, which=which: self.run(lambda: self.look_at(which)))
            self.focus[which] = cmds.checkBox(label="Focus", value=False, enable=False,
                                              changeCommand=lambda value, which=which: self.run(
                                                  lambda: self.set_focus(which, bool(value))))
            cmds.button(label="Recentre", command=lambda *_, which=which: self.run(lambda: self.recenter(which)))
            cmds.text(label="")
            cmds.setParent("..")
        cmds.setParent("..")

    # --------------------------------------------------------------- actions

    def run(self, action):
        """Run a tool; its error, if any, goes to the Live tab's message."""
        try:
            action()
            self.refresh(tools=True)
        except Exception as error:  # A tool's failure is reported, never raised into Maya's UI.
            self.live.notice = str(error) or type(error).__name__
            try:
                self.refresh(tools=True)
            except Exception:
                pass

    def toggle(self):
        if self.live.client is None:
            self.live.connect(cmds.intField(self.port, query=True, value=True))
        else:
            self.live.disconnect()
        self.refresh()

    def add(self):
        selected = cmds.ls(selection=True, long=True, type="transform") or []
        for shape in cmds.ls(selection=True, long=True, type="mesh") or []:
            selected += cmds.listRelatives(shape, parent=True, fullPath=True) or []
        if not self.live.add(selected):
            self.live.message = self.live.message or "Select mesh objects to add"
        self.refresh()

    def _selected(self):
        return self._selected_from(self._entries or self.live.rows())

    def _selected_from(self, entries):
        indices = cmds.textScrollList(self.list, query=True, selectIndexedItem=True) or []
        return [entries[index - 1][0] for index in indices if 0 < index <= len(entries)]

    def _one(self):
        selected = self._selected()
        if len(selected) != 1:
            raise ValueError("Select one mesh in the list")
        return selected[0]

    def select_in_scene(self):
        nodes = [node for node in (self.live.node(identity) for identity in self._selected()) if node]
        if nodes:
            cmds.select(nodes, replace=True)

    def _lod_identity(self):
        selected = self._selected()
        return selected[0] if len(selected) == 1 else None

    def load_lods(self):
        identity = self._lod_identity()
        enabled = identity is not None
        node = self.live.node(identity) if enabled else None
        cmds.text(self.lod_name, edit=True,
                  label=(node.rsplit("|", 1)[-1] + " (High)") if node else "Select one mesh in the Live list.")
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

    def _unit(self):
        from . import capture
        return capture.scene_unit(self.live.units(), [n for n in (self.live.node(i) for i in self.live.sources) if n])

    def create_rig(self):
        from . import capture, rig
        group, shirt = rig.create(capture.up_axis(), self._unit(), cmds.checkBox(self.with_shirt, query=True, value=True))
        if shirt:
            # Like the Blender starter: the T-shirt joins the live list.
            self.live.add([shirt])
        self.live.notice = "Created " + group.rsplit("|", 1)[-1]

    def sort_layers(self):
        from . import build, rig, rigs
        root = rigs.resolve()
        with build.undoable("Aurora SL layers"):
            rig.sort_layers(root, visible=rig_rules.VISIBLE_LAYERS)
        self.live.notice = "Sorted " + root.rsplit("|", 1)[-1] + " into SL layers"

    def show_layer(self, layer, visible):
        from . import build, rig
        with build.undoable("Aurora layer"):
            rig.show(layer, visible)

    def all_layers(self, visible):
        from . import build, rig
        with build.undoable("Aurora layers"):
            for layer, _shown in rig.layers():
                rig.show(layer, visible)

    def _weighted(self, meshes):
        """Weighted meshes join the live list, paused entries stay paused."""
        self.live.add(meshes)
        self.live.touch()

    def automatic(self):
        from . import rigs, weights
        meshes = rigs.meshes()
        root = rigs.resolve()
        mode = WEIGHT_MODES[cmds.optionMenu(self.weight_mode, query=True, value=True)]
        self._weighted(weights.automatic(meshes, root, mode))
        self.live.notice = "Weighted {} mesh(es) to {}".format(len(meshes), root.rsplit("|", 1)[-1])

    def pick_source(self):
        from . import rigs
        meshes = rigs.meshes()
        if meshes:
            cmds.textField(self.source, edit=True, text=meshes[0])

    def transfer(self):
        from . import rigs, weights
        source = cmds.textField(self.source, query=True, text=True).strip()
        if not source or not cmds.objExists(source):
            raise ValueError("Choose a weight source mesh")
        targets = weights.transfer(cmds.ls(source, long=True)[0], rigs.meshes(),
                                   cmds.optionMenu(self.nearest, query=True, select=True) == 2,
                                   cmds.floatField(self.max_distance, query=True, value=True))
        self._weighted(targets)
        self.live.notice = "Transferred weights to {} mesh(es)".format(len(targets))

    def normalize(self):
        from . import rigs, weights
        self._weighted(weights.normalize(rigs.meshes()))
        self.live.notice = "Normalized to 4 weights"

    def root_cog(self):
        from . import controls
        controls.root_cog()
        self.live.notice = "Root and COG added"

    def ik(self):
        from . import controls
        controls.ik()
        self.live.notice = "IK controls added (off); switch each limb on below"

    def set_ik(self, limb, enabled):
        from . import controls, rigs
        controls.set_ik(rigs.resolve(), limb, enabled)

    def look_at(self, which):
        from . import controls
        controls.look_at(which=which)
        self.live.notice = which + " eyes look-at target added (focus off)"

    def set_focus(self, which, enabled):
        from . import controls, rigs
        controls.set_focus(rigs.resolve(), which, enabled)

    def recenter(self, which):
        from . import controls, rigs
        controls.recenter(rigs.resolve(), which)

    # --------------------------------------------------------------- refresh

    def closed(self):
        if self.refresh in self.live.listeners:
            self.live.listeners.remove(self.refresh)

    def refresh(self, tools=False):
        if not cmds.window(WINDOW, exists=True):
            self.closed()
            return
        connected = self.live.client is not None
        cmds.button(self.connect, edit=True, label="Disconnect" if connected else "Connect")
        cmds.intField(self.port, edit=True, enable=not connected)
        status = self.live.status()
        if cmds.text(self.status, query=True, label=True) != status:
            cmds.text(self.status, edit=True, label=status)
        entries = self.live.rows()
        rows = [name + ("  —  " + message if message else "") for _identity, name, message in entries]
        if rows != self._rows:
            # Keep the selected entries, not their row numbers: grouping
            # reorders the rows.
            selected = set(self._selected_from(self._entries or []))
            cmds.textScrollList(self.list, edit=True, removeAll=True)
            for row in rows:
                cmds.textScrollList(self.list, edit=True, append=row)
            for index, (identity, _name, _message) in enumerate(entries, 1):
                if identity in selected:
                    cmds.textScrollList(self.list, edit=True, selectIndexedItem=index)
            self._rows, self._entries = rows, entries
        unit = self.live.unit
        text = "" if unit is None else "1 Maya cm = {:g} m, {}-up".format(unit, capture_up())
        if cmds.text(self.unit_text, query=True, label=True) != text:
            cmds.text(self.unit_text, edit=True, label=text)
        message = "\n".join(text for text in (self.live.message, self.live.notice) if text)
        if cmds.text(self.message, query=True, label=True) != message:
            cmds.text(self.message, edit=True, label=message)
        if tools:
            self._refresh_tools()

    def _refresh_tools(self):
        from . import controls, rig, rigs
        layers = rig.layers()
        if layers != self._layers:
            for child in cmds.columnLayout(self.layer_box, query=True, childArray=True) or []:
                cmds.deleteUI(child)
            for layer, visible in layers:
                cmds.checkBox(parent=self.layer_box, label=layer[3:].replace("_", " "), value=visible,
                              changeCommand=lambda value, layer=layer: self.run(lambda: self.show_layer(layer, bool(value))))
            self._layers = layers
        try:
            root = rigs.resolve()
        except ValueError:
            root = None
        state = controls.ik_state(root) if root else {}
        for limb, box in self.limbs.items():
            cmds.checkBox(box, edit=True, enable=limb in state, value=state.get(limb, False))
        for which, box in self.focus.items():
            focused = controls.focus_state(root, which) if root else None
            cmds.checkBox(box, edit=True, enable=focused is not None, value=bool(focused))


_window = None


def show():
    global _window
    _window = Window()
    return _window
