"""The live session: synchronized sources, change tracking and the update loop.

Everything here runs on Maya's main thread. The transport worker
(`client.py`, shared with the Blender plug-in) never touches Maya: it receives
detached snapshots only. Edits are noticed through dirty-plug callbacks on the
nodes a source depends on, then captured after a short quiet time, at most
once per update interval, like the Blender plug-in's depsgraph handler.
"""

import time
import uuid

import maya.api.OpenMaya as om
from maya import cmds

from . import capture, client, convert, lods, materials, shape

SOURCES_KEY = "auroraLiveMayaSources"
BIND_KEY = "auroraLiveMayaBindAxes"
UNITS_KEY = "auroraLiveMayaUnits"
UNIT_MODES = ("auto", "cm", "m")
LODS_KEY = "auroraLiveMayaLods"
PORT, AUTO, INTERVAL = "auroraLiveMayaPort", "auroraLiveMayaAuto", "auroraLiveMayaInterval"
DEFAULT_PORT, DEFAULT_INTERVAL = 17655, 2.0
EDIT_QUIET_SECONDS = 0.15
TICK_MS = 100
# Joint movement dirties a skinCluster's `matrix` plugs and, through them, its
# output: a pose, not an edit. An edit before the skin dirties its `input`.
POSE_PLUGS = ("matrix[", "outputGeometry")


def option(name, default):
    return cmds.optionVar(query=name) if cmds.optionVar(exists=name) else default


def set_option(name, value):
    if isinstance(value, bool) or isinstance(value, int):
        cmds.optionVar(intValue=(name, int(value)))
    elif isinstance(value, float):
        cmds.optionVar(floatValue=(name, value))
    else:
        cmds.optionVar(stringValue=(name, str(value)))


class Session:
    def __init__(self):
        self.client = None
        self.sources = []  # Maya node UUIDs, as stored in the scene
        self.errors = {}  # node UUID -> message
        self.dirty = True
        self.forced = False
        self.last_change = 0.0
        self.last_capture = 0.0
        self.revision = 0
        self.last_content = None
        self.message = ""
        self.callbacks = []
        self.watched = set()
        self.posed = set()  # skinned shapes: their dirt is a pose
        self.timer = None
        self.listeners = []
        self.jobs = []
        self._rewatch_later = False
        self.unit = None  # metres per Maya unit of the last capture
        self.load()

    # ----------------------------------------------------------- sources

    def load(self):
        """Read the synchronized list saved in the scene."""
        values = cmds.fileInfo(SOURCES_KEY, query=True)
        # Node UUIDs hold no comma: a plain list needs no quoting in the file.
        self.sources = [value for value in (values[0].split(",") if values else []) if value][:32]
        self.errors, self.dirty = {}, True
        self._rewatch()

    def save(self):
        cmds.fileInfo(SOURCES_KEY, ",".join(self.sources))

    def bind_axes(self):
        values = cmds.fileInfo(BIND_KEY, query=True)
        return bool(values and values[0] == "1")

    def set_bind_axes(self, enabled):
        cmds.fileInfo(BIND_KEY, "1" if enabled else "0")
        self.touch()

    def units(self):
        """The scene's unit choice: auto (from the rig), cm or m."""
        values = cmds.fileInfo(UNITS_KEY, query=True)
        return values[0] if values and values[0] in UNIT_MODES else "auto"

    def set_units(self, mode):
        if mode in UNIT_MODES:
            cmds.fileInfo(UNITS_KEY, mode)
            self.touch()

    def lod_settings(self, identity):
        """The Import / LOD settings of one source, per level."""
        values = cmds.fileInfo(LODS_KEY, query=True)
        stored = convert.lod_settings_parse(values[0] if values else "").get(identity, {})
        return {level: stored.get(level) or convert.lod_default(level) for level, _ratio in convert.LOD_LEVELS}

    def set_lod_settings(self, identity, levels):
        values = cmds.fileInfo(LODS_KEY, query=True)
        stored = convert.lod_settings_parse(values[0] if values else "")
        stored = {key: value for key, value in stored.items() if key in self.sources}
        stored[identity] = levels
        cmds.fileInfo(LODS_KEY, convert.lod_settings_format(stored))

    def node(self, identity):
        """The transform of a stored id, or None while it is missing."""
        nodes = cmds.ls(identity, long=True) or []
        return nodes[0] if nodes else None

    def add(self, transforms):
        added = False
        for transform in transforms:
            if capture.shape_of(transform) is None:
                continue
            identity = (cmds.ls(transform, uuid=True) or [None])[0]
            if identity and identity not in self.sources:
                if len(self.sources) >= 32:
                    self.message = "Select at most 32 mesh objects"
                    break
                self.sources.append(identity)
                added = True
        if added:
            self.save()
            self.touch()
        return added

    def remove(self, identities):
        self.sources = [identity for identity in self.sources if identity not in identities]
        for identity in identities:
            self.errors.pop(identity, None)
        self.save()
        self.touch()

    def rows(self):
        """(id, display name, message) of each synchronized source."""
        result = []
        for identity in self.sources:
            node = self.node(identity)
            name = node.rsplit("|", 1)[-1] if node else "(missing)"
            result.append((identity, name, self.errors.get(identity, "")))
        return result

    # ------------------------------------------------------ connection

    def connect(self, port):
        self.disconnect()
        set_option(PORT, int(port))
        # Import / LOD and shape creation; applying a shape stays Blender's.
        self.client = client.Client(int(port), capabilities=("import-lods", "shape-transfer"))
        self.revision, self.last_content, self.forced = 0, None, True
        self._start_timer()

    def disconnect(self):
        if self.client is not None:
            self.client.close()
            self.client = None
        self.last_content = None

    def status(self):
        if self.client is None:
            return "Not connected"
        text, _joints = self.client.state()
        return text

    # ---------------------------------------------------------- tracking

    def touch(self):
        self.dirty = True
        self.last_change = time.monotonic()

    def update_now(self):
        self.forced = True
        materials.invalidate()

    def _dirty_plug(self, node, plug, *_args):
        if node.hasFn(om.MFn.kMesh) and om.MFnDagNode(node).fullPathName() in self.posed:
            return
        name = plug.partialName(includeNodeName=False, useLongNames=True)
        if om.MFnDependencyNode(plug.node()).typeName == "skinCluster" and name.startswith(POSE_PLUGS):
            return
        self.touch()

    def _connection(self, message, plug, _other, *_args):
        if message & (om.MNodeMessage.kConnectionMade | om.MNodeMessage.kConnectionBroken):
            self.touch()
            self._rewatch_later = True

    def _nodes(self):
        """Every node a source's capture reads."""
        nodes = set()
        self.posed = set()
        for identity in self.sources:
            transform = self.node(identity)
            if not transform:
                continue
            shape = capture.shape_of(transform)
            if not shape:
                continue
            skin = capture.skin_of(shape)
            if skin:
                # The skin's input changes with every edit before it; the shape,
                # its transform and the rig move with each pose, which a
                # skinned capture ignores. The shape stays watched for its
                # material assignments only.
                nodes.update([skin, shape])
                self.posed.add(shape)
            else:
                # A static capture keeps the world rotation and scale, and a
                # linkset the offsets: the transform and every ancestor count.
                nodes.update([shape, transform] + _ancestors(transform))
            for group in set(cmds.listConnections(shape, type="shadingEngine") or []):
                nodes.update(materials.history(group))
        return nodes

    def _rewatch(self):
        self._rewatch_later = False
        try:
            nodes = self._nodes()
        except RuntimeError:
            nodes = set()
        if nodes == self.watched:
            return
        self._clear_callbacks()
        self.watched = nodes
        for name in nodes:
            selection = om.MSelectionList()
            try:
                selection.add(name)
                node = selection.getDependNode(0)
                self.callbacks.append(om.MNodeMessage.addNodeDirtyPlugCallback(node, self._dirty_plug))
                if node.hasFn(om.MFn.kMesh):
                    self.callbacks.append(om.MNodeMessage.addAttributeChangedCallback(node, self._connection))
            except RuntimeError:
                continue

    def _clear_callbacks(self):
        if self.callbacks:
            try:
                om.MMessage.removeCallbacks(self.callbacks)
            except RuntimeError:
                # Nodes of a closed scene take their callbacks with them.
                for callback in self.callbacks:
                    try:
                        om.MMessage.removeCallback(callback)
                    except RuntimeError:
                        pass
        self.callbacks = []
        self.watched = set()

    # -------------------------------------------------------------- loop

    def _start_timer(self):
        if self.timer is not None:
            return
        try:
            from PySide6.QtCore import QTimer
        except ImportError:
            from PySide2.QtCore import QTimer
        self.timer = QTimer()
        self.timer.timeout.connect(self.tick)
        self.timer.start(TICK_MS)

    def close(self):
        self.disconnect()
        if self.timer is not None:
            self.timer.stop()
            self.timer = None
        self._clear_callbacks()
        for job in self.jobs:
            if cmds.scriptJob(exists=job):
                cmds.scriptJob(kill=job, force=True)
        self.jobs = []

    def tick(self):
        try:
            self._tick()
        finally:
            for listener in list(self.listeners):
                try:
                    listener()
                except Exception:
                    self.listeners.remove(listener)

    def _tick(self):
        if self.client is None:
            return
        if self._rewatch_later:
            self._rewatch()
        _status, supported = self.client.state()
        if not supported:
            return
        request = self.client.take_import()
        if request is not None:
            self._import(request, supported)
        received = self.client.take_shape()
        if received is not None:
            self._shape(received)
        now = time.monotonic()
        auto = bool(option(AUTO, 1))
        interval = max(0.1, float(option(INTERVAL, DEFAULT_INTERVAL)))
        due = auto and self.dirty and now - self.last_capture >= interval and now - self.last_change >= EDIT_QUIET_SECONDS
        if not (self.forced or due):
            self._issues()
            return
        forced, self.forced, self.dirty = self.forced, False, False
        try:
            value = self.snapshot(supported, self.client.rest_axes(), self.revision + 1)
            content = {key: item for key, item in value.items() if key != "revision"}
            if forced or content != self.last_content:
                self.revision += 1
                self.client.submit(value)
                self.last_content = content
        except Exception as error:  # Any failure must show, not stop the timer.
            self.message = str(error) or type(error).__name__
        finally:
            self.last_capture = time.monotonic()
            try:
                self._rewatch()
            except Exception as error:
                self.message = str(error) or type(error).__name__

    def _import(self, request, supported):
        """Answer Aurora's Import button with the four prepared levels."""
        try:
            identity = next((key for key in self.sources if _uuid_for(key) == str(request.get("source"))), None)
            transform = self.node(identity) if identity else None
            if transform is None:
                raise ValueError("The requested mesh is no longer in this scene's live list")
            unit = capture.scene_unit(self.units(), [self.node(key) for key in self.sources if self.node(key)])
            base = convert.base_frame(capture.up_axis(), unit)
            result = lods.prepare(request, transform, self.lod_settings(identity), supported, self.client.rest_axes(),
                                  base, self.bind_axes())
        except Exception as error:  # Aurora waits for an answer: always send one.
            result = dict(request, error=(str(error) or type(error).__name__)[:512])
        self.client.import_result(result)

    def _shape(self, payload):
        """Build the skeleton Aurora sent with the worn shape; Aurora reports
        success only once this answers."""
        identity = payload.get("id", "") if isinstance(payload, dict) else ""
        try:
            unit = capture.scene_unit(self.units(), [self.node(key) for key in self.sources if self.node(key)])
            group = shape.create(payload, capture.up_axis(), unit)
            self.message = "Shape received: " + group.rsplit("|", 1)[-1]
            self.client.shape_result(identity)
        except Exception as error:  # Aurora waits for an answer: always send one.
            self.message = str(error) or type(error).__name__
            self.client.shape_result(identity, self.message[:512])

    def _issues(self):
        revision, issues = self.client.issues()
        if revision == self.revision and issues:
            for identity in self.sources:
                message = issues.get(_uuid_for(identity))
                if message:
                    self.errors[identity] = message

    def snapshot(self, supported, rest_axes, revision):
        """Capture every source; a failing one is skipped with its reason."""
        images = convert.Images()
        meshes, corners, errors, nodes = [], 0, {}, {}
        axes_mode = self.bind_axes()
        unit = capture.scene_unit(self.units(), [self.node(identity) for identity in self.sources if self.node(identity)])
        base = convert.base_frame(capture.up_axis(), unit)
        self.unit = unit
        for identity in self.sources:
            transform = self.node(identity)
            if transform:
                nodes[identity] = transform
        static = {}
        for identity, transform in nodes.items():
            mark = images.mark()
            warnings = []
            try:
                mesh, count = capture.mesh(transform, supported, rest_axes, images, warnings, axes_mode, base)
                if corners + count > convert.MAX_CORNERS:
                    raise ValueError("Snapshot exceeds 600,000 mesh corners")
            except (ValueError, RuntimeError) as error:
                images.rollback(mark)
                errors[identity] = str(error) or type(error).__name__
                continue
            if warnings:
                errors[identity] = "Textures skipped, plain colour shown: " + warnings[0]
            mesh["source_id"] = _uuid_for(identity)
            corners += count
            meshes.append(mesh)
            if not mesh["joints"]:
                static[identity] = transform
        # Maya parenting makes linksets: a static mesh under another
        # synchronized static mesh is a prim of that one's linkset.
        by_path = {transform: identity for identity, transform in static.items()}
        parents = {}
        for identity, transform in static.items():
            parent = None
            for ancestor in _ancestors(transform):
                if ancestor in by_path:
                    parent = by_path[ancestor]
                    break
            parents[identity] = parent
        roots = convert.static_roots(parents)
        for mesh in meshes:
            identity = next(key for key in nodes if _uuid_for(key) == mesh["source_id"])
            root = roots.get(identity)
            if root is not None and root != identity:
                mesh["link"] = {"parent": _uuid_for(root), "offset": capture.link_offset(static[identity], static[root], base)}
        self.errors = errors
        self.message = ""
        return {"protocol": 1, "revision": revision, "meshes": meshes, "images": images.images}


def _ancestors(path):
    parts = path.split("|")
    return ["|".join(parts[:index]) for index in range(len(parts) - 1, 1, -1)]


def _uuid_for(identity):
    try:
        value = uuid.UUID(identity)
        if value.int:
            return str(value)
    except ValueError:
        pass
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "maya:" + identity))


_session = None


def get():
    global _session
    if _session is None:
        _session = Session()
        for event in ("SceneOpened", "NewSceneOpened"):
            _session.jobs.append(cmds.scriptJob(event=(event, _session.load)))
    return _session
