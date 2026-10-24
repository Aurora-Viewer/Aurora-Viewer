"""Prepare a frozen four-LOD import on disposable rest-mesh copies."""
import re
import uuid
import bpy
from bpy.props import EnumProperty, FloatProperty, IntProperty, PointerProperty, StringProperty
from . import exporter, materials

LEVELS = (("medium", 50.0), ("low", 25.0), ("lowest", 12.5))


class AURORA_PG_lod(bpy.types.PropertyGroup):
    mode: EnumProperty(name="LOD source", items=[
        ("FALLBACK", "Existing or automatic", "Use the assigned or named object; generate only when none exists"),
        ("EXISTING", "Existing object", "Require the assigned or named mesh"),
        ("AUTO", "Automatic", "Generate a disposable reduced copy of the High mesh"),
    ], default="FALLBACK")
    object: PointerProperty(name="LOD object", type=bpy.types.Object, poll=lambda _self, obj: obj.type == "MESH")
    suffix: StringProperty(name="Name suffix", default="", maxlen=48)
    budget: EnumProperty(name="Reduction target", items=[("RATIO", "Percentage", "Percentage of High triangles"),
                                                       ("TRIANGLES", "Triangles", "Maximum triangle target")])
    ratio: FloatProperty(name="Keep (%)", default=25, min=0.1, max=100, precision=1)
    triangles: IntProperty(name="Triangle target", default=100, min=1, max=200000)


def initialize(entry):
    if not entry.lod_initialized:
        for name, ratio in LEVELS:
            spec = getattr(entry, name)
            spec.ratio, spec.suffix = ratio, "_" + name
        entry.lod_initialized = True


def existing(scene, high, spec):
    if spec.object is not None:
        if spec.object == high or scene.objects.get(spec.object.name) != spec.object:
            raise ValueError("Choose a different LOD mesh in the current scene")
        return spec.object
    stem = re.sub(r"[._-]high$", "", high.name, flags=re.IGNORECASE)
    target = (stem + spec.suffix).casefold()
    found = [obj for obj in scene.objects if obj != high and obj.type == "MESH" and obj.name.casefold() == target]
    if len(found) > 1:
        raise ValueError("Ambiguous LOD object name: " + stem + spec.suffix)
    return found[0] if found else None


def _capture(obj, supported, forward, axes, capture):
    mesh = exporter._mesh(obj, supported, forward, axes, capture, include_slots=True)
    return mesh, mesh.pop("_slots")


def _reduced(context, high, target):
    clone = high.copy()
    clone.data = high.data.copy()
    clone.parent = None
    clone.constraints.clear()
    clone.animation_data_clear()
    clone.matrix_world = high.matrix_world
    context.scene.collection.objects.link(clone)
    clone.hide_viewport = False
    clone.hide_set(False)
    data = clone.data
    try:
        for mod in list(clone.modifiers):
            if mod.type == "ARMATURE":
                clone.modifiers.remove(mod)
        context.view_layer.update()
        graph = context.evaluated_depsgraph_get()
        evaluated = clone.evaluated_get(graph)
        # Bake current keys/modifiers at rest before decimation; originals and
        # their keys are never edited. Evaluated vertex-group interpolation is
        # retained by Blender's mesh copy/Decimate path.
        rest = bpy.data.meshes.new_from_object(evaluated, preserve_all_data_layers=True, depsgraph=graph)
        clone.data = rest
        if data.users == 0:
            bpy.data.meshes.remove(data)
        clone.modifiers.clear()
        rest.calc_loop_triangles()
        triangles = len(rest.loop_triangles)
        if triangles > target:
            mod = clone.modifiers.new("Aurora import LOD", "DECIMATE")
            mod.ratio = max(0.000001, min(1.0, target / triangles))
            mod.use_collapse_triangulate = True
        # Reattach the rig solely for rest capture; exporter removes it on its
        # own temporary copy, so the current pose is never baked into a LOD.
        if exporter.armature_for(high):
            clone.modifiers.new("Aurora import rig", "ARMATURE").object = exporter.armature_for(high)
        context.view_layer.update()
        return clone
    except Exception:
        data = clone.data
        bpy.data.objects.remove(clone, do_unlink=True)
        if data.users == 0:
            bpy.data.meshes.remove(data)
        raise


def _palette(meshes):
    names, binds = [], {}
    for mesh in meshes:
        for name, inverse in zip(mesh["joints"], mesh["inverse_bind"]):
            if name in binds and max(abs(a-b) for a, b in zip(inverse, binds[name])) > 0.0001:
                raise ValueError("LOD bind matrices disagree with High")
            if name not in binds:
                names.append(name)
                binds[name] = inverse
    if len(names) > 110:
        raise ValueError("Import LODs exceed 110 avatar joints")
    for mesh in meshes:
        remap = [names.index(name) for name in mesh["joints"]]
        for face in mesh["faces"]:
            for vertex in face["vertices"]:
                vertex["joints"] = [remap[index] for index in vertex["joints"]] if remap else [0] * 4
        mesh["joints"], mesh["inverse_bind"] = names, [binds[name] for name in names]


def prepare(context, request, supported, axes):
    from . import tracking
    uuid.UUID(request["id"])
    source = str(uuid.UUID(request["source"]))
    entry = next((entry for entry in context.scene.aurora_live.objects if entry.source_id == source), None)
    if entry is None or not tracking.valid(context.scene, entry.object):
        raise ValueError("The requested source is no longer in this scene's live list")
    initialize(entry)
    high = entry.object
    capture = materials.Capture()
    forward = context.scene.aurora_live.forward
    high_mesh, high_slots = _capture(high, supported, forward, axes, capture)
    high_count = sum(len(face["indices"]) // 3 for face in high_mesh["faces"])
    high_materials = [high.material_slots[index].material if index < len(high.material_slots) else None for index in high_slots]
    levels = {"high": {"mesh": high_mesh, "slots": list(range(len(high_slots))), "origin": high.name}}
    for name, _ratio in LEVELS:
        spec = getattr(entry, name)
        obj = existing(context.scene, high, spec) if spec.mode != "AUTO" else None
        if obj is None and spec.mode == "EXISTING":
            raise ValueError("Missing LOD object: " + high.name + " / " + name)
        temporary = None
        try:
            if obj is None:
                target = spec.triangles if spec.budget == "TRIANGLES" else max(1, round(high_count * spec.ratio / 100))
                temporary = obj = _reduced(context, high, target)
            if exporter.armature_for(obj) != exporter.armature_for(high):
                raise ValueError("LOD objects must use the same avatar rig as High")
            mesh, slots = _capture(obj, supported, forward, axes, capture)
            mapping = []
            for index in slots:
                material = obj.material_slots[index].material if index < len(obj.material_slots) else None
                if material not in high_materials:
                    raise ValueError("LOD materials must match High's material slots")
                same_slot = high_slots.index(index) if index in high_slots else None
                if same_slot is not None and high_materials[same_slot] == material:
                    mapping.append(same_slot)
                elif high_materials.count(material) == 1:
                    mapping.append(high_materials.index(material))
                else:
                    raise ValueError("Ambiguous LOD material slots; retain High's slot order")
            if len(set(mapping)) != len(mapping):
                raise ValueError("Duplicate material slots in a LOD are ambiguous")
            mesh["name"] = high.name
            mesh["source_id"] = source
            levels[name] = {"mesh": mesh, "slots": mapping, "origin": "Automatic" if temporary else obj.name}
        finally:
            if temporary:
                data = temporary.data
                bpy.data.objects.remove(temporary, do_unlink=True)
                if data.users == 0:
                    bpy.data.meshes.remove(data)
    high_mesh["source_id"] = source
    ordered = [levels[name] for name in ("lowest", "low", "medium", "high")]
    if sum(len(face["vertices"]) for level in ordered for face in level["mesh"]["faces"]) > exporter.MAX_CORNERS:
        raise ValueError("Prepared LODs exceed 600,000 mesh corners")
    _palette([level["mesh"] for level in ordered])
    context.view_layer.update()
    return {"id": request["id"], "source": source, "levels": ordered, "images": capture.images}


def draw(layout, context):
    prefs = context.scene.aurora_live
    if not 0 <= prefs.object_index < len(prefs.objects):
        return
    entry = prefs.objects[prefs.object_index]
    layout.prop(prefs, "show_lods", text="Import / LOD", icon="MOD_DECIM")
    if not prefs.show_lods:
        return
    layout.label(text=entry.name, icon="MESH_DATA")
    for name, _ratio in LEVELS:
        spec = getattr(entry, name)
        box = layout.box()
        box.label(text=name.title())
        box.prop(spec, "mode", text="")
        if spec.mode != "AUTO":
            box.prop(spec, "object")
            if spec.object is None:
                box.prop(spec, "suffix")
        if spec.mode != "EXISTING":
            row = box.row(align=True)
            row.prop(spec, "budget", text="")
            row.prop(spec, "ratio" if spec.budget == "RATIO" else "triangles")
    layout.label(text="Sources stay unchanged; inspect each LOD in Aurora.", icon="INFO")


CLASSES = (AURORA_PG_lod,)
