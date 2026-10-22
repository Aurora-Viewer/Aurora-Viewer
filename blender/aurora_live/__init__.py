"""Blender editing to Aurora's animated avatar, without uploading assets."""

bl_info = {
    "name": "Aurora Live Mesh", "author": "Aurora Viewer contributors",
    "version": (0, 1, 0), "blender": (2, 93, 0),
    "location": "View3D > Sidebar > Aurora", "category": "3D View",
    "description": "Preview rigged mesh edits live on your avatar in Aurora Viewer",
}

import time
import bpy
from bpy.app.handlers import persistent
from bpy.props import EnumProperty, FloatProperty, IntProperty, PointerProperty
from .client import Client
from . import exporter

_client = None
_objects = []
_dirty = True
_exporting = False
_revision = 0
_last_capture = 0.0
_error = ""


def _selected():
    return [obj for obj in bpy.context.selected_objects if obj.type == "MESH"]


@persistent
def _changed(_scene, depsgraph):
    global _dirty
    if not _client or _exporting:
        return
    watched = set()
    for obj in _objects:
        try:
            watched.update((obj, obj.data))
            armature = obj.find_armature()
            if armature:
                watched.update((armature, armature.data))
            for slot in obj.material_slots:
                if slot.material:
                    watched.add(slot.material)
                    if slot.material.node_tree:
                        watched.add(slot.material.node_tree)
        except ReferenceError:
            _dirty = True
    if any(update.id.original in watched for update in depsgraph.updates):
        _dirty = True


@persistent
def _load_pre(_unused):
    _stop()


def _stop():
    global _client, _objects, _error
    if _client:
        _client.close()
    _client = None
    _objects = []
    _error = ""
    if bpy.app.timers.is_registered(_tick):
        bpy.app.timers.unregister(_tick)


def _tick():
    global _dirty, _revision, _last_capture, _exporting, _error, _objects
    if _client is None:
        return None
    status, supported = _client.state()
    valid = []
    for obj in _objects:
        try:
            if obj.name in bpy.data.objects:
                valid.append(obj)
        except ReferenceError:
            pass
    if len(valid) != len(_objects):
        _objects = valid
        _dirty = True
    prefs = bpy.context.scene.aurora_live
    if supported and _dirty and time.monotonic() - _last_capture >= prefs.interval:
        _exporting = True
        _dirty = False
        try:
            _revision += 1
            value = exporter.snapshot(_objects, supported, _revision, prefs.forward, _client.rest_axes())
            _client.submit(value)
            _error = ""
        except (ValueError, RuntimeError, ReferenceError, OverflowError) as error:
            _error = str(error)
        finally:
            _last_capture = time.monotonic()
            _exporting = False
    # A redraw shows connection errors even when the user is not editing.
    for window in bpy.context.window_manager.windows:
        for area in window.screen.areas:
            if area.type == "VIEW_3D":
                area.tag_redraw()
    return 0.1


def _settings_changed(_self, _context):
    global _dirty
    _dirty = True


class AURORA_PG_live(bpy.types.PropertyGroup):
    port: IntProperty(name="Aurora port", default=17655, min=1024, max=65535)
    interval: FloatProperty(name="Update interval", default=0.2, min=0.1, max=5.0, subtype="TIME", update=_settings_changed)
    forward: EnumProperty(name="Coordinates", items=[
        ("NATIVE", "SL / Z up", "Use the rig's existing SL coordinate axes"),
        ("NEGATIVE_Y", "Blender -Y forward", "Rotate Blender -Y to SL +X"),
    ], default="NATIVE", update=_settings_changed)


class AURORA_OT_connect(bpy.types.Operator):
    bl_idname = "aurora.live_connect"
    bl_label = "Connect to Aurora"

    def execute(self, context):
        global _client, _objects, _dirty, _last_capture
        selected = _selected()
        if not selected:
            self.report({"ERROR"}, "Select one or more rigged meshes first")
            return {"CANCELLED"}
        _stop()
        _objects = selected
        _dirty = True
        _last_capture = 0.0
        _client = Client(context.scene.aurora_live.port)
        bpy.app.timers.register(_tick, first_interval=0.1)
        return {"FINISHED"}


class AURORA_OT_disconnect(bpy.types.Operator):
    bl_idname = "aurora.live_disconnect"
    bl_label = "Disconnect"

    def execute(self, _context):
        _stop()
        return {"FINISHED"}


class AURORA_OT_selection(bpy.types.Operator):
    bl_idname = "aurora.live_selection"
    bl_label = "Use selected meshes"

    def execute(self, _context):
        global _objects, _dirty, _error
        _objects = _selected()
        _dirty = True
        _error = ""
        return {"FINISHED"}


class AURORA_OT_refresh(bpy.types.Operator):
    bl_idname = "aurora.live_refresh"
    bl_label = "Send now"

    def execute(self, _context):
        global _dirty, _last_capture
        _dirty = True
        _last_capture = 0.0
        return {"FINISHED"}


class AURORA_PT_live(bpy.types.Panel):
    bl_label = "Aurora Live Mesh"
    bl_idname = "AURORA_PT_live"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Aurora"

    def draw(self, context):
        layout = self.layout
        prefs = context.scene.aurora_live
        row = layout.row()
        row.enabled = _client is None
        row.prop(prefs, "port")
        layout.prop(prefs, "interval")
        layout.prop(prefs, "forward")
        if _client:
            status, _supported = _client.state()
            layout.label(text=status, translate=False)
            layout.operator("aurora.live_disconnect", icon="UNLINKED")
            row = layout.row(align=True)
            row.operator("aurora.live_selection")
            row.operator("aurora.live_refresh", icon="FILE_REFRESH")
            layout.label(text="{} tracked meshes".format(len(_objects)))
        else:
            layout.operator("aurora.live_connect", icon="LINKED")
        if _error:
            box = layout.box()
            box.alert = True
            box.label(text=_error, icon="ERROR", translate=False)
        layout.label(text="Enable Blender Live in Aurora first.")
        layout.label(text="Local preview · no asset upload")


_classes = (AURORA_PG_live, AURORA_OT_connect, AURORA_OT_disconnect, AURORA_OT_selection, AURORA_OT_refresh, AURORA_PT_live)

_translations = {"fr_FR": {("*", key): value for key, value in {
    "Aurora Live Mesh": "Maillage Aurora Live", "Connect to Aurora": "Connecter à Aurora",
    "Disconnect": "Déconnecter", "Use selected meshes": "Utiliser la sélection",
    "Send now": "Envoyer maintenant", "Aurora port": "Port Aurora", "Update interval": "Intervalle de mise à jour",
    "Coordinates": "Coordonnées", "SL / Z up": "SL / Z vertical", "Blender -Y forward": "Blender -Y vers l’avant",
    "Enable Blender Live in Aurora first.": "Activez d’abord Blender Live dans Aurora.",
    "Local preview · no asset upload": "Aperçu local · sans téléversement",
}.items()}}


def register():
    bpy.app.translations.register(__name__, _translations)
    for cls in _classes:
        bpy.utils.register_class(cls)
    bpy.types.Scene.aurora_live = PointerProperty(type=AURORA_PG_live)
    bpy.app.handlers.depsgraph_update_post.append(_changed)
    bpy.app.handlers.load_pre.append(_load_pre)


def unregister():
    _stop()
    bpy.app.translations.unregister(__name__)
    for handlers, callback in ((bpy.app.handlers.depsgraph_update_post, _changed), (bpy.app.handlers.load_pre, _load_pre)):
        if callback in handlers:
            handlers.remove(callback)
    if hasattr(bpy.types.Scene, "aurora_live"):
        del bpy.types.Scene.aurora_live
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)
