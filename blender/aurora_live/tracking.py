"""Scene-owned synchronization entries; object pointers survive renaming/saving."""
import bpy
import uuid
from bpy.props import BoolProperty, IntProperty, PointerProperty, StringProperty
from . import lods

on_change = None


def changed(_self=None, _context=None):
    if on_change:
        on_change()


def valid(scene, obj):
    try:
        return obj is not None and obj.type == "MESH" and scene.objects.get(obj.name) == obj
    except ReferenceError:
        return False


def prune(scene):
    prefs = scene.aurora_live
    missing = [index for index, entry in enumerate(prefs.objects) if not valid(scene, entry.object)]
    for index in reversed(missing):
        prefs.objects.remove(index)
    prefs.object_index = min(prefs.object_index, max(0, len(prefs.objects) - 1))
    if missing:
        changed()


def active(scene):
    return [entry.object for entry in scene.aurora_live.objects if entry.enabled and valid(scene, entry.object)]


def source_ids(scene):
    """Persistent entry identities survive renaming, saving and list reordering."""
    result = {}
    seen = set()
    for entry in scene.aurora_live.objects:
        if not valid(scene, entry.object):
            continue
        try:
            key = uuid.UUID(entry.source_id)
        except (ValueError, AttributeError):
            key = None
        if key is None or not key.int or key in seen:
            key = uuid.uuid4()
            entry.source_id = str(key)
        seen.add(key)
        result[entry.object.as_pointer()] = str(key)
    return result


def add(scene, objects):
    prune(scene)
    prefs = scene.aurora_live
    seen = {entry.object.as_pointer() for entry in prefs.objects}
    added = 0
    for obj in objects:
        if valid(scene, obj) and obj.as_pointer() not in seen:
            entry = prefs.objects.add()
            entry.object = obj
            lods.initialize(entry)
            seen.add(obj.as_pointer())
            added += 1
    if added:
        prefs.object_index = len(prefs.objects) - 1
        changed()
    return added


class AURORA_PG_sync_object(bpy.types.PropertyGroup):
    medium: PointerProperty(type=lods.AURORA_PG_lod)
    low: PointerProperty(type=lods.AURORA_PG_lod)
    lowest: PointerProperty(type=lods.AURORA_PG_lod)
    lod_initialized: BoolProperty(default=False, options={"HIDDEN"})
    object: PointerProperty(type=bpy.types.Object)
    source_id: StringProperty(options={"HIDDEN"})
    name: StringProperty(get=lambda self: self.object.name if self.object else "")
    enabled: BoolProperty(name="Synchronize", default=True, update=changed,
                          description="Include this object in Aurora's live preview")


class AURORA_OT_selection(bpy.types.Operator):
    bl_idname = "aurora.live_selection"
    bl_label = "Add selected meshes"
    bl_description = "Add the current selection without replacing listed objects or enabling paused entries"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return any(obj.type == "MESH" for obj in context.selected_objects)

    def execute(self, context):
        add(context.scene, context.selected_objects)
        return {"FINISHED"}


class AURORA_OT_remove(bpy.types.Operator):
    bl_idname = "aurora.live_remove"
    bl_label = "Remove from synchronization"
    bl_description = "Remove this entry from Aurora's preview list; keep the Blender object"
    bl_options = {"REGISTER", "UNDO"}
    index: IntProperty(default=0, min=0)

    def execute(self, context):
        prefs = context.scene.aurora_live
        if self.index >= len(prefs.objects):
            return {"CANCELLED"}
        prefs.objects.remove(self.index)
        prefs.object_index = min(prefs.object_index, max(0, len(prefs.objects) - 1))
        changed()
        return {"FINISHED"}


class AURORA_UL_sync_objects(bpy.types.UIList):
    def draw_item(self, context, layout, data, item, icon, active_data, active_propname, index=0, flt_flag=0):
        row = layout.row(align=True)
        row.prop(item, "enabled", text="")
        label = row.row()
        label.enabled = item.enabled
        if valid(context.scene, item.object):
            label.label(text=item.object.name, icon="MESH_DATA", translate=False)
        else:
            label.label(text="Missing object", icon="ERROR")
        row.operator("aurora.live_remove", text="", icon="X", emboss=False).index = index


CLASSES = (AURORA_PG_sync_object, AURORA_OT_selection, AURORA_OT_remove, AURORA_UL_sync_objects)
