"""Scene-owned synchronization entries; object pointers survive renaming/saving."""
import bpy
import uuid
from bpy.app.translations import pgettext_iface
from bpy.props import BoolProperty, IntProperty, PointerProperty, StringProperty
from . import exporter, lods

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
    prefs.object_index = min(prefs.object_index, len(prefs.objects) - 1)
    repaired = normalize_groups(scene)
    if missing or repaired:
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


def is_static(scene, obj):
    return valid(scene, obj) and exporter.armature_for(obj) is None


def normalize_groups(scene):
    groups, repaired = {}, False
    for entry in scene.aurora_live.objects:
        if entry.group_id:
            if is_static(scene, entry.object):
                groups.setdefault(entry.group_id, []).append(entry)
            else:
                entry.group_id, entry.group_root = "", False
                repaired = True
    for entries in groups.values():
        root = next((entry for entry in entries if entry.group_root), entries[0])
        for entry in entries:
            if len(entries) == 1:
                entry.group_id, entry.group_root, entry.independent = "", False, True
                repaired = True
            elif entry.group_root != (entry == root):
                entry.group_root = entry == root
                repaired = True
    return repaired


def link_roots(scene):
    groups = {}
    for entry in scene.aurora_live.objects:
        if entry.group_id and is_static(scene, entry.object):
            if entry.group_id not in groups or entry.group_root:
                groups[entry.group_id] = entry.object
    result = {}
    for entry in scene.aurora_live.objects:
        if not is_static(scene, entry.object):
            continue
        root = groups.get(entry.group_id)
        if root is not None or entry.independent:
            result[entry.object.as_pointer()] = root if root != entry.object else None
    return result


def signature(scene):
    # RNA update callbacks are not replayed by Undo/Redo.
    return tuple((entry.object.as_pointer(), entry.group_id, entry.group_root, entry.independent)
                 for entry in scene.aurora_live.objects if valid(scene, entry.object))


def rows(scene):
    entries = scene.aurora_live.objects
    objects = [entry.object for entry in entries if valid(scene, entry.object)]
    roots = exporter.static_roots(objects, link_roots(scene))
    indices = {entry.object.as_pointer(): index for index, entry in enumerate(entries) if valid(scene, entry.object)}
    families = {}
    for index, entry in enumerate(entries):
        root = roots.get(entry.object.as_pointer()) if valid(scene, entry.object) else None
        root_index = indices[root.as_pointer()] if root is not None else index
        families.setdefault(root_index, []).append(index)
    result = []
    for root_index, members in sorted(families.items()):
        ordered = [root_index] + [index for index in members if index != root_index]
        result.extend((index, root_index, number, len(ordered)) for number, index in enumerate(ordered, 1))
    return result


def families(scene):
    result = {}
    for index, root, _number, _count in rows(scene):
        entry = scene.aurora_live.objects[index]
        if is_static(scene, entry.object):
            result.setdefault(root, []).append(entry)
    return list(result.values())


def marked(scene):
    return [entry for entry in scene.aurora_live.objects if entry.marked and is_static(scene, entry.object)]


def select_entry(scene, index, extend=False, range_select=False):
    prefs = scene.aurora_live
    if not 0 <= index < len(prefs.objects) or not valid(scene, prefs.objects[index].object):
        return False
    source_ids(scene)
    if range_select:
        order = [item[0] for item in rows(scene)]
        anchor = next((i for i, entry in enumerate(prefs.objects) if entry.source_id == prefs.selection_anchor), index)
        start, end = sorted((order.index(anchor), order.index(index)))
        selected = set(order[start:end + 1])
        for i, entry in enumerate(prefs.objects):
            entry.marked = i in selected or (extend and entry.marked)
    elif extend:
        prefs.objects[index].marked = not prefs.objects[index].marked
        prefs.selection_anchor = prefs.objects[index].source_id
    else:
        for i, entry in enumerate(prefs.objects):
            entry.marked = i == index
        prefs.selection_anchor = prefs.objects[index].source_id
    prefs.object_index = index if prefs.objects[index].marked else next(
        (i for i, entry in enumerate(prefs.objects) if entry.marked), -1)
    return True


def set_group(entries, root):
    group_id = str(uuid.uuid4()) if len(entries) > 1 else ""
    for entry in entries:
        entry.group_id = group_id
        entry.group_root = bool(group_id) and entry == root
        entry.independent = True


def regroup(scene, entries, root=None):
    if len(entries) < 2:
        return False
    root = root if root in entries else entries[0]
    selected = {entry.object.as_pointer() for entry in entries}
    for family in families(scene):
        remaining = [entry for entry in family if entry.object.as_pointer() not in selected]
        if len(remaining) != len(family) and remaining:
            set_group(remaining, remaining[0])
    set_group(entries, root)
    changed()
    return True


def ungroup(scene, entries, whole_roots=False):
    selected = {entry.object.as_pointer() for entry in entries}
    current = families(scene)
    if whole_roots:
        for family in current:
            if family[0].object.as_pointer() in selected:
                selected.update(entry.object.as_pointer() for entry in family)
        entries = [entry for entry in scene.aurora_live.objects if is_static(scene, entry.object)
                   and entry.object.as_pointer() in selected]
    for family in current:
        remaining = [entry for entry in family if entry.object.as_pointer() not in selected]
        if len(remaining) != len(family) and remaining:
            set_group(remaining, remaining[0])
    for entry in entries:
        set_group([entry], entry)
    changed()


def add(scene, objects, root=None):
    prune(scene)
    prefs = scene.aurora_live
    objects = [obj for obj in objects if valid(scene, obj)]
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
        selected = {obj.as_pointer() for obj in objects}
        entries = []
        for index, entry in enumerate(prefs.objects):
            entry.marked = entry.object.as_pointer() in selected
            if entry.marked and is_static(scene, entry.object):
                entries.append(entry)
            if entry.object == root:
                prefs.object_index = index
        chosen = next((entry for entry in entries if entry.object == root), None)
        regroup(scene, entries, chosen)
        source_ids(scene)
        prefs.selection_anchor = prefs.objects[prefs.object_index].source_id
        changed()
    return added


class AURORA_PG_sync_object(bpy.types.PropertyGroup):
    medium: PointerProperty(type=lods.AURORA_PG_lod)
    low: PointerProperty(type=lods.AURORA_PG_lod)
    lowest: PointerProperty(type=lods.AURORA_PG_lod)
    lod_initialized: BoolProperty(default=False, options={"HIDDEN"})
    object: PointerProperty(type=bpy.types.Object)
    source_id: StringProperty(options={"HIDDEN"})
    group_id: StringProperty(options={"HIDDEN"})
    group_root: BoolProperty(default=False, options={"HIDDEN"})
    independent: BoolProperty(default=False, options={"HIDDEN"})
    marked: BoolProperty(default=False, options={"HIDDEN", "SKIP_SAVE"})
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
        add(context.scene, context.selected_objects, context.view_layer.objects.active)
        return {"FINISHED"}


def focused(context):
    prefs = context.scene.aurora_live
    if 0 <= prefs.object_index < len(prefs.objects):
        entry = prefs.objects[prefs.object_index]
        if is_static(context.scene, entry.object):
            return entry
    return None


class AURORA_OT_pick(bpy.types.Operator):
    bl_idname = "aurora.live_pick"
    bl_label = "Select synchronized object"
    bl_description = "Click to select; Ctrl-click to toggle; Shift-click to select a range"
    bl_options = {"INTERNAL"}
    index: IntProperty(default=0, min=0)
    extend: BoolProperty(default=False, options={"HIDDEN", "SKIP_SAVE"})
    range_select: BoolProperty(default=False, options={"HIDDEN", "SKIP_SAVE"})

    def invoke(self, context, event):
        self.extend = event.ctrl or event.oskey
        self.range_select = event.shift
        return self.execute(context)

    def execute(self, context):
        return {"FINISHED"} if select_entry(context.scene, self.index, self.extend, self.range_select) else {"CANCELLED"}


class AURORA_OT_group(bpy.types.Operator):
    bl_idname = "aurora.live_group"
    bl_label = "Group"
    bl_description = "Group selected entries; the active selected entry becomes the root"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return len(marked(context.scene)) >= 2

    def execute(self, context):
        return {"FINISHED"} if regroup(context.scene, marked(context.scene), focused(context)) else {"CANCELLED"}


class AURORA_OT_ungroup(bpy.types.Operator):
    bl_idname = "aurora.live_ungroup"
    bl_label = "Ungroup"
    bl_description = "Ungroup selected children, or the whole linkset when its root is selected"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return bool(marked(context.scene))

    def execute(self, context):
        ungroup(context.scene, marked(context.scene), whole_roots=True)
        return {"FINISHED"}


class AURORA_OT_root(bpy.types.Operator):
    bl_idname = "aurora.live_root"
    bl_label = "Make root"
    bl_description = "Use the highlighted entry as the root of its linkset"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        entry = focused(context)
        return entry is not None and any(entry in family and len(family) > 1 for family in families(context.scene))

    def execute(self, context):
        entry = focused(context)
        family = next((family for family in families(context.scene) if entry in family), [])
        return {"FINISHED"} if regroup(context.scene, family, entry) else {"CANCELLED"}


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
        entry = prefs.objects[self.index]
        if is_static(context.scene, entry.object):
            ungroup(context.scene, [entry])
        prefs.objects.remove(self.index)
        prefs.object_index = min(prefs.object_index, len(prefs.objects) - 1)
        normalize_groups(context.scene)
        changed()
        return {"FINISHED"}


class AURORA_UL_sync_objects(bpy.types.UIList):
    def filter_items(self, context, data, propname):
        ordered = rows(context.scene)
        positions = {index: position for position, (index, _root, _number, _count) in enumerate(ordered)}
        return [], [positions[index] for index in range(len(getattr(data, propname)))]

    def draw_item(self, context, layout, data, item, icon, active_data, active_propname, index=0, flt_flag=0):
        row = layout.row(align=True)
        row.prop(item, "enabled", text="", icon="LINKED" if item.enabled else "UNLINKED", emboss=False)
        label = row.row()
        label.operator_context = "INVOKE_DEFAULT"
        if valid(context.scene, item.object):
            _index, _root, number, count = next(value for value in rows(context.scene) if value[0] == index)
            name = item.object.name
            if count > 1:
                if number == 1:
                    name = pgettext_iface("Root #1 · {name}").format(name=name)
                else:
                    label.label(text="", icon="BLANK1")
                    name = pgettext_iface("#{number} · {name}").format(number=number, name=name)
            button = label.operator("aurora.live_pick", text=name,
                                    icon="OBJECT_ORIGIN" if count > 1 and number == 1 else "MESH_DATA",
                                    depress=item.marked, translate=False)
            button.index = index
        else:
            label.label(text="Missing object", icon="ERROR")
        row.operator("aurora.live_remove", text="", icon="X", emboss=False).index = index


CLASSES = (AURORA_PG_sync_object, AURORA_OT_selection, AURORA_OT_pick, AURORA_OT_group, AURORA_OT_ungroup, AURORA_OT_root,
           AURORA_OT_remove, AURORA_UL_sync_objects)
