"""Blender editing to Aurora's animated avatar, without uploading assets."""

bl_info = {
    "name": "Aurora LiveMesh", "author": "Aurora Viewer contributors",
    "version": (0, 14, 6), "blender": (2, 93, 0),
    "location": "View3D > Sidebar > Aurora", "category": "3D View",
    "description": "Preview rigged and static mesh edits live in Aurora Viewer",
}

_reloading = "bpy" in globals()
import importlib
import time
import traceback
import bpy
from bpy.app.handlers import persistent
from bpy.app.translations import pgettext_iface
from bpy.props import BoolProperty, CollectionProperty, EnumProperty, FloatProperty, IntProperty, PointerProperty, StringProperty
from . import compat, client, materials, weights, starter, exporter, lods, tracking, handles, rigging, skinning, posing, tools_ui, shape_apply, shape_transfer
if _reloading:
    # Blender reloads the package during an in-place extension upgrade. Reload
    # dependencies first so the old exporter/client cannot survive that upgrade.
    for _module in (compat, client, materials, weights, starter, exporter, lods, tracking, handles, rigging, skinning, posing, tools_ui, shape_apply, shape_transfer):
        importlib.reload(_module)
Client = client.Client

_client = None
_object_signature = ()
_dirty = True
_geometry_dirty = True
_exporting = False
_revision = 0
_last_capture = 0.0
_force_capture = False
_last_snapshot = None
_error = ""
_material_signature = ()
_incoming_shape = None
_last_change = 0.0
_display_signature = None
EDIT_QUIET_SECONDS = 0.15


def _selected():
    return [obj for obj in bpy.context.selected_objects if obj.type == "MESH"]


@persistent
def _changed(_scene, depsgraph):
    global _dirty, _last_change, _geometry_dirty
    if not _client or _exporting:
        return
    watched = set()
    geometry = set()
    for obj in tracking.active(bpy.context.scene):
        try:
            ids = {obj, obj.data}
            parent = obj.parent
            while parent:
                ids.add(parent)
                parent = parent.parent
            armature = obj.find_armature()
            if armature:
                ids.update((armature, armature.data))
            watched.update(ids)
            geometry.update(ids)
            for slot in obj.material_slots:
                if slot.material:
                    watched.add(slot.material)
                    if slot.material.node_tree:
                        for tree in materials.trees(slot.material):
                            watched.add(tree)
                            watched.update(node.image for node in tree.nodes if node.type == "TEX_IMAGE" and node.image)
        except ReferenceError:
            _dirty = True
            _geometry_dirty = True
    if any(update.id.original in watched for update in depsgraph.updates):
        _dirty = True
        _last_change = time.monotonic()
        if any(update.id.original in geometry for update in depsgraph.updates):
            _geometry_dirty = True
        if any(isinstance(update.id.original, bpy.types.Image) for update in depsgraph.updates):
            materials.invalidate()


@persistent
def _load_pre(_unused):
    _stop()


@persistent
def _refresh_handles(_unused=None):
    for scene in bpy.data.scenes:
        if not any(entry.marked for entry in scene.aurora_live.objects):
            scene.aurora_live.object_index = -1
        for entry in scene.aurora_live.objects:
            lods.initialize(entry)
    for obj in list(bpy.data.objects):
        if obj.type == "ARMATURE":
            handles.apply(obj)


def _handle_size_changed(obj, _context):
    handles.apply(obj)


def _stop():
    global _client, _error, _material_signature, _object_signature, _force_capture, _last_capture, _last_snapshot
    global _last_change, _display_signature
    global _geometry_dirty
    if _client:
        _client.close()
    _client = None
    _object_signature = ()
    _error = ""
    _material_signature = ()
    _force_capture = False
    _last_capture = 0.0
    _last_snapshot = None
    _last_change, _display_signature = 0.0, None
    _geometry_dirty = True
    materials.invalidate()
    if bpy.app.timers.is_registered(_tick):
        bpy.app.timers.unregister(_tick)


def _tick():
    global _error, _dirty, _exporting, _last_capture
    if _exporting:
        return 0.1
    try:
        return _update()
    except Exception as error:
        # An uncaught exception makes Blender unregister the timer. Keep the
        # connection recoverable, including errors outside the mesh capture.
        message = pgettext_iface("LiveMesh update failed: {error}").format(error=str(error) or type(error).__name__)
        if _error != message:
            traceback.print_exc()
        _error, _dirty, _exporting = message, True, False
        _last_capture = time.monotonic()
        return 0.1
    finally:
        _redraw_status()


def _redraw_status(context=None):
    global _display_signature
    context = context or bpy.context
    try:
        status = _client.state()[0] if _client else ""
    except Exception:
        status = ""
    prefs = getattr(context.scene, "aurora_live", None)
    signature = (status, _error, _revision,
                 tuple((entry.source_id, entry.name, entry.enabled, entry.error) for entry in prefs.objects) if prefs else ())
    if signature == _display_signature:
        return
    _display_signature = signature
    # Connection feedback belongs to the sidebar; redrawing every 3D viewport
    # at 10 Hz needlessly wakes complex scenes even when nothing is edited.
    for window in context.window_manager.windows:
        for area in window.screen.areas:
            if area.type == "VIEW_3D":
                for region in area.regions:
                    if region.type == "UI":
                        region.tag_redraw()


def _editing(context=None):
    context = context or bpy.context
    windows = context.window_manager.windows
    if any(window.screen.is_animation_playing for window in windows):
        return False
    if context.mode in {"EDIT_MESH", "SCULPT", "PAINT_WEIGHT", "PAINT_VERTEX", "PAINT_TEXTURE"}:
        return True
    return any(operator.bl_idname.startswith("TRANSFORM_OT_")
               for window in windows for operator in getattr(window, "modal_operators", ()))


def _update():
    global _dirty, _revision, _last_capture, _exporting, _error, _material_signature, _object_signature
    global _force_capture, _last_snapshot, _incoming_shape
    global _geometry_dirty
    if _client is None:
        return None
    status, supported = _client.state()
    scene = bpy.context.scene
    request = _client.take_import(ready=bpy.context.mode == "OBJECT")
    if request is not None:
        _exporting = True
        try:
            _client.import_result(lods.prepare(bpy.context, request, supported, _client.rest_axes()))
        except (ValueError, RuntimeError, KeyError, TypeError, OverflowError) as error:
            _client.import_result(dict(request, error=pgettext_iface(str(error))[:512]))
        finally:
            _exporting = False
    _incoming_shape = _client.take_shape(ready=not bpy.context.object or bpy.context.object.mode == "OBJECT",
                                         pose=bpy.context.mode == "POSE")
    if _incoming_shape is not None:
        identity = _incoming_shape.get("id", "") if isinstance(_incoming_shape, dict) else ""
        try:
            if bpy.ops.aurora.receive_shape("EXEC_DEFAULT", True) != {"FINISHED"}:
                raise ValueError("Shape import was cancelled")
            _client.shape_result(identity)
            _request_capture()
        except (ValueError, RuntimeError, KeyError, TypeError, OverflowError) as error:
            _client.shape_result(identity, str(error)[:512])
        finally:
            _incoming_shape = None
    tracking.prune(scene)
    objects = tracking.active(scene)
    prefs = scene.aurora_live
    # Polling also sees Undo/Redo, scene switches and names changed while idle.
    membership = (scene.as_pointer(), scene.unit_settings.scale_length, prefs.forward,
                  tuple((obj.as_pointer(), obj.name) for obj in objects), tracking.signature(scene))
    if membership != _object_signature:
        _object_signature = membership
        _request_capture()
    polling_error = ""
    try:
        signature = materials.signature(objects)
    except Exception as error:
        # A node's unsupported RNA value must not stop geometry updates.
        signature = None
        _dirty = True
        polling_error = pgettext_iface("LiveMesh material check failed: {error}").format(error=str(error) or type(error).__name__)
    if signature != _material_signature:
        _material_signature = signature
        _dirty = True
    painting = bpy.context.mode == "PAINT_TEXTURE"
    if painting:
        _dirty = True
    now = time.monotonic()
    due = (prefs.auto_update and _dirty and now - _last_capture >= prefs.interval
           and (not _editing() or now - _last_change >= EDIT_QUIET_SECONDS))
    if supported and (_force_capture or due):
        forced, _force_capture = _force_capture, False
        _exporting = True
        _dirty = False
        try:
            if painting:
                materials.invalidate()
            errors = {}
            packed = hasattr(_client, "features") and "packed-geometry" in _client.features()
            identities = tracking.source_ids(scene)
            value = (exporter.material_snapshot(objects, _last_snapshot, identities, _revision + 1, errors)
                     if packed and not forced and not _geometry_dirty else None)
            if value is None:
                value = exporter.snapshot(objects, supported, _revision + 1, prefs.forward, _client.rest_axes(),
                                          identities, tracking.link_roots(scene), errors=errors, packed=packed)
            _geometry_dirty = False
            messages = [polling_error] if polling_error else []
            for entry in prefs.objects:
                message = errors.get(entry.object.as_pointer(), "") if tracking.valid(scene, entry.object) else ""
                prefix = entry.name + ": "
                if message.startswith(prefix):
                    message = message[len(prefix):]
                text = pgettext_iface(message) if message else ""
                if entry.error != text:
                    entry.error = text
                if entry.error:
                    messages.append(prefix + entry.error)
            # Texture painting can change pixels without a depsgraph event.
            # Compare the prepared payload, sharing its buffers with the worker,
            # so idle paint ticks and unrelated notifications send no revisions.
            content = {key: item for key, item in value.items() if key != "revision"}
            if forced or content != _last_snapshot:
                _revision += 1
                _client.submit(value)
                _last_snapshot = content
            _error = "\n".join(messages)
        except (ValueError, RuntimeError, ReferenceError, OverflowError) as error:
            _error = pgettext_iface(str(error))
            _geometry_dirty = True
        finally:
            _last_capture = time.monotonic()
            _exporting = False
    if hasattr(_client, "issues"):
        revision, issues = _client.issues()
        if revision == _revision:
            captured = {mesh.get("source_id") for mesh in (_last_snapshot or {}).get("meshes", [])}
            for entry in prefs.objects:
                if entry.enabled and entry.source_id in captured:
                    message = issues.get(entry.source_id, "")
                    text = pgettext_iface(message) if message else ""
                    if entry.error != text:
                        entry.error = text
    return 0.1


def _settings_changed(_self, _context):
    global _dirty, _geometry_dirty
    _dirty = True
    _geometry_dirty = True


def _request_capture():
    global _dirty, _error, _geometry_dirty
    _dirty, _error = True, ""
    _geometry_dirty = True
    # Images may have changed while their object was paused or disconnected.
    materials.invalidate()


def _weights_changed(context, objects):
    if _client:
        tracking.add(context.scene, objects)
        _request_capture()


class AURORA_OT_receive_shape(bpy.types.Operator):
    bl_idname = "aurora.receive_shape"
    bl_label = "Receive avatar shape"
    bl_options = {"REGISTER", "UNDO", "INTERNAL"}

    def execute(self, context):
        if _incoming_shape is None:
            return {"CANCELLED"}
        try:
            shape_transfer.apply(context, _incoming_shape)
        except (ValueError, RuntimeError, KeyError, TypeError, OverflowError) as error:
            self.report({"ERROR"}, pgettext_iface(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_PG_live(bpy.types.PropertyGroup):
    show_lods: BoolProperty(name="Import / LOD", default=False)
    objects: CollectionProperty(type=tracking.AURORA_PG_sync_object)
    object_index: IntProperty(default=-1, min=-1, options={"SKIP_SAVE"})
    selection_anchor: StringProperty(default="", options={"SKIP_SAVE"})
    rig: PointerProperty(name="Avatar rig", type=bpy.types.Object, poll=lambda _self, obj: obj.type == "ARMATURE")
    weight_source: PointerProperty(name="Weight source", type=bpy.types.Object, poll=lambda _self, obj: obj.type == "MESH")
    transfer_method: EnumProperty(name="Weight mapping", items=[
        ("SURFACE", "Nearest surface", "Interpolate weights on the closest source triangle in world space"),
        ("VERTEX", "Nearest vertex", "Copy weights from the closest source vertex in world space"),
    ], default="SURFACE")
    transfer_distance: FloatProperty(name="Max distance (0 = unlimited)", default=0, min=0, subtype="DISTANCE", unit="LENGTH")
    weight_scope: EnumProperty(name="Auto-weight bones", items=[
        ("BODY", "SL body", "Use the classic body bones; skip overlapping Bento and collision bones"),
        ("ALL", "All deform bones", "Include deform bones except collision volumes and overlapping Bento spine joints"),
    ], default="BODY")
    starter_tshirt: BoolProperty(name="Add rigged test T-shirt", default=True,
                                description="Create a weighted, editable T-shirt on the new SL skeleton")
    port: IntProperty(name="Aurora port", default=17655, min=1024, max=65535)
    auto_update: BoolProperty(name="Automatic updates", default=True,
                              description="Send changed objects and materials at the chosen interval; disable for manual updates")
    interval: FloatProperty(name="Interval (seconds)", default=2.0, min=0.1, max=3600.0, soft_max=60.0, precision=2,
                            description="Minimum time in seconds between automatic updates; unchanged content is not sent")
    forward: EnumProperty(name="Coordinates", items=[
        ("AUTO", "Automatic (Avastar / SL)", "Detect each rig's rest axes; create new rigs facing Blender -Y", 0, 2),
        ("NATIVE", "SL / Z up", "Use the rig's existing SL coordinate axes", 0, 0),
        ("NEGATIVE_Y", "Blender -Y forward", "Rotate Blender -Y to SL +X", 0, 1),
    ], default="AUTO", update=_settings_changed)


class AURORA_OT_starter(bpy.types.Operator):
    bl_idname = "aurora.create_sl_rig"
    bl_label = "Create SL avatar rig"
    bl_description = "Create the complete Bento and Fitted Mesh skeleton at the 3D cursor"
    bl_options = {"REGISTER", "UNDO"}

    with_tshirt: BoolProperty(name="Add rigged test T-shirt", default=True)

    @classmethod
    def poll(cls, context):
        return context.mode == "OBJECT"

    def execute(self, context):
        try:
            _rig, shirt = starter.create(context, self.with_tshirt, context.scene.aurora_live.forward)
            context.scene.aurora_live.rig = _rig
        except (ValueError, RuntimeError, OSError) as error:
            self.report({"ERROR"}, str(error))
            return {"CANCELLED"}
        if shirt:
            tracking.add(context.scene, [shirt])
        self.report({"INFO"}, pgettext_iface("Created 133 Bento bones and 26 Fitted Mesh bones"))
        return {"FINISHED"}


class AURORA_OT_connect(bpy.types.Operator):
    bl_idname = "aurora.live_connect"
    bl_label = "Connect to Aurora"

    def execute(self, context):
        global _client, _force_capture
        tracking.prune(context.scene)
        if not context.scene.aurora_live.objects:
            tracking.add(context.scene, _selected(), context.view_layer.objects.active)
        _stop()
        _request_capture()
        _force_capture = True
        _client = Client(context.scene.aurora_live.port)
        bpy.app.timers.register(_tick, first_interval=0.1)
        return {"FINISHED"}


class AURORA_OT_weights(bpy.types.Operator):
    bl_idname = "aurora.auto_weights"
    bl_label = "Recalculate automatic weights"
    bl_description = "Replace bone weights using Blender's automatic heat solver; Undo restores the previous rig"
    bl_options = {"REGISTER", "UNDO"}

    @classmethod
    def poll(cls, context):
        return context.mode in {"OBJECT", "PAINT_WEIGHT"} and any(obj.type == "MESH" for obj in context.selected_objects)

    def execute(self, context):
        selected = _selected()
        prefs = context.scene.aurora_live
        try:
            weights.rebind(context, selected, prefs.rig, prefs.weight_scope)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, pgettext_iface(str(error)))
            return {"CANCELLED"}
        if _client:
            tracking.add(context.scene, selected)
            _request_capture()
        self.report({"INFO"}, pgettext_iface("Automatic weights recalculated"))
        return {"FINISHED"}


class AURORA_OT_disconnect(bpy.types.Operator):
    bl_idname = "aurora.live_disconnect"
    bl_label = "Disconnect"

    def execute(self, _context):
        _stop()
        return {"FINISHED"}


class AURORA_OT_refresh(bpy.types.Operator):
    bl_idname = "aurora.live_refresh"
    bl_label = "Update now"
    bl_description = "Send the latest enabled objects and materials immediately, even in manual mode"

    @classmethod
    def poll(cls, _context):
        return _client is not None and not _exporting

    def execute(self, _context):
        global _force_capture
        _request_capture()
        _force_capture = True
        # Capture on this click; keep the request pending during the handshake.
        _tick()
        return {"FINISHED"}


class AURORA_PT_live(bpy.types.Panel):
    bl_label = "Aurora"
    bl_idname = "AURORA_PT_live"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Aurora"

    def draw(self, context):
        layout = self.layout
        for values in (("LIVE", "RIG"), ("IK", "WEIGHTS")):
            row = layout.row(align=True)
            row.scale_y = 1.15
            for value in values:
                row.prop_enum(context.window_manager, "aurora_live_tab", value)
        if tools_ui.tab(context) != "LIVE":
            layout.label(text="Avatar rig", icon="ARMATURE_DATA")
            layout.prop(context.scene.aurora_live, "rig", text="")
        layout.separator(factor=0.5)
        current = tools_ui.tab(context)
        if current == "LIVE":
            _draw_live(layout, context)
            settings = tools_ui.foldout(layout, context.window_manager, "aurora_live_show_connection", "Connection settings")
            if settings:
                _draw_connection(settings, context)
        elif current == "RIG":
            tools_ui.draw_rig(layout, context)
            starter_box = tools_ui.foldout(layout, context.window_manager, "aurora_live_show_starter", "Create an SL avatar")
            if starter_box:
                _draw_starter(starter_box, context)
        elif current == "IK":
            tools_ui.draw_ik(layout, context)
        else:
            for title, draw in (("Automatic weights", tools_ui.draw_automatic),
                                ("Weight transfer", tools_ui.draw_transfer), ("Weight cleanup", tools_ui.draw_clean)):
                box = layout.box()
                box.label(text=title)
                draw(box, context)


def _draw_live(layout, context):
    prefs = context.scene.aurora_live
    if _client:
        status, _supported = _client.state()
        row = layout.row(align=True)
        row.label(text=pgettext_iface(status), translate=False)
        if _error and not any(entry.error for entry in prefs.objects):
            row.operator_context = "INVOKE_DEFAULT"
            row.operator("aurora.live_error", text="", icon="ERROR").message = _error
        layout.operator("aurora.live_disconnect", icon="UNLINKED")
    else:
        layout.operator("aurora.live_connect", icon="LINKED")
    box = layout.box()
    box.prop(prefs, "auto_update")
    row = box.row()
    row.enabled = prefs.auto_update
    row.prop(prefs, "interval")
    box.label(text="Only changed content is sent." if prefs.auto_update else "Manual updates only.", icon="INFO")
    box.operator("aurora.live_refresh", icon="FILE_REFRESH")
    layout.label(text="Synchronized objects", icon="OUTLINER_OB_MESH")
    layout.operator("aurora.live_selection", icon="ADD")
    if prefs.objects:
        layout.template_list("AURORA_UL_sync_objects", "", prefs, "objects", prefs, "object_index", rows=4, maxrows=8)
        row = layout.row(align=True)
        row.operator("aurora.live_group", icon="LINKED")
        row.operator("aurora.live_ungroup", icon="UNLINKED")
        layout.operator("aurora.live_root", icon="OBJECT_ORIGIN")
        layout.label(text=pgettext_iface("{selected} selected · {enabled}/{total} enabled").format(
            selected=sum(entry.marked for entry in prefs.objects),
            enabled=len(tracking.active(context.scene)), total=len(prefs.objects)), translate=False)
    else:
        layout.label(text="Add selected meshes to start syncing.")
    lods.draw(layout, context)
    layout.label(text="Enable LiveMesh in Aurora first.")
    layout.label(text="Local preview · no asset upload")


def _draw_connection(layout, context):
    prefs = context.scene.aurora_live
    row = layout.row()
    row.enabled = _client is None
    row.prop(prefs, "port")
    layout.prop(prefs, "forward")


def _draw_starter(layout, context):
    prefs = context.scene.aurora_live
    layout.prop(prefs, "starter_tshirt")
    layout.prop(prefs, "forward")
    layout.operator("aurora.create_sl_rig", icon="ADD").with_tshirt = prefs.starter_tshirt
    if context.mode != "OBJECT":
        layout.label(text="Switch to Object Mode to create a rig.", icon="INFO")

_classes = lods.CLASSES + tracking.CLASSES + (AURORA_PG_live, AURORA_OT_receive_shape, AURORA_OT_starter, AURORA_OT_weights, AURORA_OT_connect,
                             AURORA_OT_disconnect, AURORA_OT_refresh, AURORA_PT_live) + tools_ui.CLASSES

_translations = {"fr_FR": {(context, key): value for context in ("*", "Operator") for key, value in {
    "Synchronization error": "Erreur de synchronisation",
    "Copy error": "Copier l’erreur", "Error copied": "Erreur copiée",
    "Copy the complete object name and synchronization error": "Copier le nom complet de l’objet et son erreur de synchronisation",
    "Show this object's synchronization error and copy its details": "Afficher l’erreur de synchronisation de cet objet et copier les détails",
    "Update Aurora Viewer to preview this large object": "Mettez à jour Aurora Viewer pour afficher cet objet volumineux",
    "This object exceeds the LiveMesh transfer limit; reduce its geometry or textures":
        "Cet objet dépasse la limite de transfert LiveMesh ; réduisez sa géométrie ou ses textures",
    "LiveMesh update failed: {error}": "Échec de la mise à jour LiveMesh : {error}",
    "LiveMesh material check failed: {error}": "Échec de la vérification des matériaux LiveMesh : {error}",
    "Connecting to Aurora…": "Connexion à Aurora…", "Connected to Aurora": "Connecté à Aurora",
    "Import / LOD": "Import / LOD", "LOD source": "Source du LOD", "LOD object": "Objet LOD",
    "Existing or automatic": "Existant sinon automatique", "Existing object": "Objet existant",
    "Automatic": "Automatique", "Name suffix": "Suffixe du nom", "Reduction target": "Objectif de réduction",
    "Percentage": "Pourcentage", "Triangles": "Triangles", "Keep (%)": "Conserver (%)",
    "Triangle target": "Objectif de triangles",
    "Use the assigned or named object; generate only when none exists":
        "Utiliser l’objet associé ou détecté par son nom ; générer uniquement s’il n’existe pas",
    "Require the assigned or named mesh": "Exiger l’objet associé ou détecté par son nom",
    "Generate a disposable reduced copy of the High mesh": "Générer une copie réduite temporaire du maillage High",
    "Percentage of High triangles": "Pourcentage des triangles du niveau High",
    "Maximum triangle target": "Objectif maximal de triangles",
    "Sources stay unchanged; inspect each LOD in Aurora.": "Sources préservées ; vérifiez chaque LOD dans Aurora.",
    "Choose a different LOD mesh in the current scene": "Choisissez un autre maillage LOD dans la scène actuelle",
    "LOD objects must use the same avatar rig as High": "Les LOD doivent utiliser le même squelette que High",
    "LOD materials must match High's material slots": "Les matériaux des LOD doivent correspondre à ceux de High",
    "LOD bind matrices disagree with High": "Les matrices de liaison des LOD diffèrent de celles de High",
    "Import LODs exceed 110 avatar joints": "Les LOD de l’import dépassent 110 articulations",
    "The requested source is no longer in this scene's live list": "L’objet demandé ne figure plus dans la liste live de cette scène",
    "Waiting for Aurora: Update Aurora Viewer to use live PBR materials":
        "Mettez à jour Aurora Viewer pour utiliser les matériaux PBR live",
    "Live materials use the mesh's active UV map": "Les matériaux live utilisent la carte UV active du maillage",
    "Use UV coordinates for live image textures": "Utilisez les coordonnées UV pour les textures image live",
    "Live UV mapping supports Z rotation only": "Le mapping UV live accepte uniquement la rotation autour de Z",
    "Use a UV Image Texture, optionally through a Point Mapping node":
        "Utilisez une texture image UV, éventuellement reliée à un nœud Mapping de type Point",
    "Live normals need a tangent-space Normal Map with strength 1":
        "Les normales live nécessitent une carte de normales en espace tangent, d’intensité 1",
    "Bake procedural shaders to UV image textures for live materials":
        "Convertissez les shaders procéduraux en textures image UV pour les matériaux live",
    "Bake advanced Principled effects to textures for Second Life PBR":
        "Convertissez les effets Principled avancés en textures pour le PBR Second Life",
    "Live materials need flat, single-image UV textures": "Les matériaux live nécessitent des textures UV planes à image unique",
    "Use Linear interpolation for live image textures": "Utilisez l’interpolation Linéaire pour les textures image live",
    "Use sRGB, Linear Rec.709 or Non-Color image color spaces":
        "Utilisez les espaces colorimétriques sRGB, Linéaire Rec.709 ou Non-couleur",
    "Live source images must be loaded and at most 16 megapixels":
        "Les images source live doivent être chargées et contenir au plus 16 mégapixels",
    "Image pixels must be finite": "Les valeurs des pixels doivent être finies",
    "Live materials exceed 16 images or 16 MiB of pixels; reduce image sizes":
        "Les matériaux live dépassent 16 images ou 16 Mio de pixels ; réduisez les dimensions des images",
    "Use a Principled BSDF surface for live PBR materials": "Utilisez une surface BSDF Principled pour les matériaux PBR live",
    "Connect the base image's Alpha output directly to Principled Alpha":
        "Reliez la sortie Alpha de l’image de couleur directement à l’entrée Alpha du BSDF Principled",
    "Roughness and metallic images must share a UV transform":
        "Les images de rugosité et de métal doivent partager la même transformation UV",
    "Add an active UV map to use live image textures": "Ajoutez une carte UV active pour utiliser les textures image live",
    "Avatar rig": "Squelette d’avatar", "Auto-weight bones": "Os à pondérer",
    "Section": "Catégorie", "Live": "Live", "Skeleton": "Squelette", "Weights": "Poids",
    "Connect and manage synchronized meshes": "Connexion et gestion des maillages synchronisés",
    "Create an SL avatar and show or hide bone parts": "Création d’un avatar SL et affichage des parties du squelette",
    "Control hands and feet with inverse kinematics": "Contrôle des mains et des pieds en cinématique inverse",
    "Calculate, transfer and normalize mesh weights": "Calcul, transfert et normalisation des poids du maillage",
    "Live preview": "Aperçu live", "Connection settings": "Réglages de connexion",
    "Create an SL avatar": "Créer un avatar SL", "Bone visibility": "Visibilité des os",
    "Create IK controls": "Créer les contrôles IK", "Hands": "Mains", "Feet": "Pieds",
    "Left": "Gauche", "Right": "Droite", "Automatic weights": "Poids automatiques",
    "Recalculate weights": "Recalculer les poids", "Weight transfer": "Transfert de poids",
    "Transfer weights": "Transférer les poids", "Weight cleanup": "Nettoyage des poids",
    "Normalize to 4 weights": "Normaliser à 4 poids",
    "Select the target meshes.": "Sélectionnez les maillages cibles.",
    "The source stays unchanged.": "Le donneur est préservé.",
    "Back / torso": "Dos / torse", "Head / neck": "Tête / cou", "Bento spine": "Colonne Bento",
    "Controls": "Contrôles", "Hand/foot IK, root/COG and eye targets": "IK des mains/pieds, Root/COG et cibles du regard",
    "Handle size": "Taille des poignées", "Refresh IK handles": "Actualiser les poignées IK",
    "Reposition poles": "Recaler les pôles",
    "Correct knee bend": "Corriger la flexion",
    "Make active knees bend toward their poles while keeping the foot targets fixed":
        "Orienter les genoux actifs vers leurs pôles en gardant les cibles des pieds fixes",
    "Enable foot IK before correcting knee bend": "Activez les IK des pieds avant de corriger la flexion",
    "Reposition elbow / knee poles": "Recaler les pôles des coudes / genoux",
    "Align poles with the current limb pose without rebuilding the rig":
        "Recaler les pôles sur la pose actuelle des membres sans reconstruire le squelette",
    "Cannot determine the avatar's facing direction": "Impossible de déterminer l’orientation de l’avatar",
    "Replace IK widgets without changing the pose, weights or solver":
        "Remplacer les poignées IK sans modifier la pose, les poids ni le solveur",
    "Root / COG": "Root / COG", "Root": "Root", "COG": "COG", "Add root / COG": "Ajouter Root / COG",
    "Look at": "Regard", "Classic eyes": "Yeux classiques", "Bento eyes": "Yeux Bento",
    "Classic": "Classiques", "Bento": "Bento",
    "Root / COG controls": "Contrôles Root / COG", "Look-at targets": "Cibles du regard",
    "Create pose controls": "Créer les contrôles de pose", "Select pose control": "Sélectionner un contrôle de pose",
    "Add non-deforming root/COG controls or an eye target without changing SL rest joints":
        "Ajouter des contrôles Root/COG ou une cible du regard sans modifier les os SL au repos",
    "Enable / disable eye focus": "Activer / désactiver le regard",
    "Follow the eye target; disabling keeps the current eye pose for manual rotation":
        "Suivre la cible ; la désactivation conserve la pose des yeux pour les orienter manuellement",
    "Center eye target": "Recentrer la cible du regard",
    "Place the eye target ahead of the current head pose": "Placer la cible devant la tête dans sa pose actuelle",
    "Control poses stay in Blender.": "Les poses restent dans Blender.",
    "This rig is missing the required SL joints": "Ce squelette ne possède pas les articulations SL nécessaires",
    "Leave Edit Mode before creating pose controls": "Quittez le mode Édition avant de créer les contrôles de pose",
    "Pose control names are already used; keep the existing rig unchanged":
        "Les noms des contrôles sont déjà utilisés ; le squelette existant est préservé",
    "These joints already have constraints; use a simple SL rig for Aurora pose controls":
        "Ces os ont déjà des contraintes ; utilisez un squelette SL simple pour les contrôles Aurora",
    "Create the eye target first": "Créez d’abord la cible du regard",
    "The eye pair has no common forward direction": "Les deux yeux n’ont pas de direction commune vers l’avant",
    "Use Pose Position and leave Edit Mode to change eye focus":
        "Utilisez la position de pose et quittez le mode Édition pour modifier le regard",
    "Create the pose control first": "Créez d’abord le contrôle de pose",
    "Advanced bones": "Os avancés", "Folded SL spine joints": "Articulations SL repliées",
    "No Bento spine bones in this rig": "Ce rig n’a pas d’os de colonne Bento",
    "Show or hide bones without changing the rig or its weights":
        "Afficher ou masquer les os sans modifier le squelette ni ses poids",
    "SL body": "Corps SL", "All deform bones": "Tous les os de déformation",
    "Recalculate automatic weights": "Recalculer les poids automatiques",
    "Replace bone weights using Blender's automatic heat solver; Undo restores the previous rig":
        "Remplacer les poids avec le calcul automatique de Blender ; Annuler restaure les poids précédents",
    "Use the classic body bones; skip overlapping Bento and collision bones":
        "Utiliser les os classiques du corps, sans les os Bento superposés ni les os de collision",
    "Include deform bones except collision volumes and overlapping Bento spine joints":
        "Inclure les os de déformation, sauf les volumes de collision et les os de colonne Bento superposés",
    "Automatic weights recalculated": "Poids automatiques recalculés",
    "Skeleton / Bento / IK": "Squelette / Bento / IK", "Weights / transfer": "Poids / transfert",
    "Weight source": "Maillage donneur", "Weight mapping": "Correspondance des poids",
    "Nearest surface": "Surface la plus proche", "Nearest vertex": "Sommet le plus proche",
    "Interpolate weights on the closest source triangle in world space":
        "Interpoler les poids sur le triangle donneur le plus proche dans l’espace global",
    "Copy weights from the closest source vertex in world space":
        "Copier les poids du sommet donneur le plus proche dans l’espace global",
    "Max distance (0 = unlimited)": "Distance max. (0 = illimitée)",
    "Show / hide bones": "Afficher / masquer les os", "All bones": "Tous les os", "Deform bones": "Os de déformation",
    "In front": "Au premier plan", "Names": "Noms", "Visible parts": "Parties visibles",
    "Show all": "Tout afficher", "Hide all": "Tout masquer", "Body": "Corps", "Arms": "Bras", "Legs": "Jambes",
    "Bento fingers": "Doigts Bento", "Bento face / eyes": "Visage / yeux Bento", "Bento wings": "Ailes Bento",
    "Bento tail": "Queue Bento", "Bento hind limbs": "Membres arrière Bento",
    "Fitted Mesh volumes": "Volumes Fitted Mesh", "IK controls": "Contrôles IK", "Other bones": "Autres os",
    "Hand / foot IK": "IK mains / pieds", "Create hand / foot IK controls": "Créer les contrôles IK mains / pieds",
    "Add eight non-deforming target and pole bones; keep SL rest bones unchanged":
        "Ajouter huit os de commande sans déformation ; conserver les os SL au repos",
    "Enable / disable IK": "Activer / désactiver l’IK", "Select IK control": "Sélectionner un contrôle IK",
    "Snap IK controls to the current pose when enabling; keep the visible pose when disabling":
        "Aligner les contrôles IK sur la pose à l’activation ; conserver la pose à la désactivation",
    "Left hand IK": "IK main gauche", "Right hand IK": "IK main droite",
    "Left foot IK": "IK pied gauche", "Right foot IK": "IK pied droit", "Target": "Cible", "Pole": "Pôle",
    "IK poses stay in Blender.": "Les poses IK restent dans Blender.",
    "Aurora animates the rest mesh.": "Aurora anime le maillage au repos.",
    "Transfer weights from source": "Transférer les poids du donneur",
    "Replace selected target bone weights from the source mesh, normalized to four influences":
        "Remplacer les poids des cibles sélectionnées depuis le donneur, normalisés à quatre influences",
    "Select target meshes; the source stays unchanged.": "Sélectionnez les cibles ; le donneur est préservé.",
    "Normalize / limit to four weights": "Normaliser / limiter à quatre poids",
    "Weights normalized to four influences": "Poids normalisés à quatre influences", "Weights transferred": "Poids transférés",
    "Choose an avatar rig first": "Choisissez d’abord un squelette d’avatar",
    "Use an editable local armature": "Utilisez une armature locale éditable",
    "This armature is missing the SL arm or leg chain": "Il manque une chaîne de bras ou de jambe SL dans cette armature",
    "The IK chain has zero length": "La chaîne IK a une longueur nulle",
    "Leave Edit Mode before creating IK controls": "Quittez le mode Édition avant de créer les contrôles IK",
    "The limb already has constraints; use a simple SL rig for Aurora IK":
        "Ce membre a déjà des contraintes ; utilisez un squelette SL simple pour l’IK Aurora",
    "An IK control name is already used by another bone": "Un nom de contrôle IK est déjà utilisé par un autre os",
    "Create Aurora IK controls first": "Créez d’abord les contrôles IK Aurora",
    "Use Pose Position and leave Edit Mode to switch IK": "Utilisez la position de pose et quittez le mode Édition pour basculer l’IK",
    "Some vertices have no transferable deform weights; original weights were kept":
        "Certains sommets n’ont aucun poids de déformation transférable ; les poids précédents sont conservés",
    "Unlock the target's bone vertex groups before replacing weights":
        "Déverrouillez les groupes d’os de la cible avant de remplacer les poids",
    "Select target meshes other than the weight source": "Sélectionnez des maillages cibles distincts du donneur",
    "Choose a mesh as the weight source": "Choisissez un maillage donneur de poids",
    "The weight source needs vertices and, for surface transfer, faces":
        "Le donneur doit avoir des sommets et, pour le transfert de surface, des faces",
    "No nearby source surface; original weights were kept": "Aucune surface donneuse proche ; les poids précédents sont conservés",
    "A target vertex exceeds the transfer distance; original weights were kept":
        "Un sommet cible dépasse la distance de transfert ; les poids précédents sont conservés",
    "No suitable deform bones in the chosen rig": "Aucun os de déformation adapté dans le squelette choisi",
    "Select one or more meshes to recalculate their weights": "Sélectionnez les maillages à repondérer",
    "Choose an avatar rig or select one armature with the meshes": "Choisissez un squelette d’avatar ou sélectionnez une armature avec les maillages",
    "Use editable objects with at most one Armature modifier": "Utilisez des objets éditables avec au plus un modificateur Armature",
    "The selected mesh has no vertices": "Le maillage sélectionné n’a aucun sommet",
    "Automatic weights failed; the original weights were kept. Check the mesh topology.":
        "Le calcul automatique a échoué ; les poids précédents sont conservés. Vérifiez la topologie du maillage.",
    "SL avatar starter": "Avatar SL de départ",
    "Create SL avatar rig": "Créer le squelette d’avatar SL",
    "Create the complete Bento and Fitted Mesh skeleton at the 3D cursor":
        "Créer le squelette Bento et Fitted Mesh complet au curseur 3D",
    "Add rigged test T-shirt": "Ajouter un T-shirt de test riggé",
    "Create a weighted, editable T-shirt on the new SL skeleton":
        "Créer un T-shirt éditable et pondéré sur le nouveau squelette SL",
    "Switch to Object Mode to create a rig.": "Passez en mode Objet pour créer un squelette.",
    "Created 133 Bento bones and 26 Fitted Mesh bones": "133 os Bento et 26 os Fitted Mesh créés",
    "Aurora LiveMesh": "Aurora LiveMesh", "Connect to Aurora": "Connecter à Aurora",
    "Disconnect": "Déconnecter", "Add selected meshes": "Ajouter la sélection",
    "Add the current selection without replacing listed objects or enabling paused entries":
        "Ajouter la sélection sans remplacer les objets listés ni réactiver les objets en pause",
    "Synchronized objects": "Objets synchronisés", "Synchronize": "Synchroniser",
    "Select synchronized object": "Sélectionner un objet synchronisé",
    "Click to select; Ctrl-click to toggle; Shift-click to select a range":
        "Cliquer pour sélectionner ; Ctrl-clic pour ajouter ou retirer ; Maj-clic pour sélectionner une plage",
    "{selected} selected · {enabled}/{total} enabled": "{selected} sélectionnés · {enabled}/{total} actifs",
    "Group": "Grouper", "Ungroup": "Dégrouper", "Make root": "Définir root",
    "Group selected entries; the active selected entry becomes the root":
        "Grouper les objets sélectionnés ; la ligne active sélectionnée devient le root",
    "Ungroup selected children, or the whole linkset when its root is selected":
        "Dégrouper les enfants sélectionnés, ou tout le linkset lorsque son root est sélectionné",
    "Use the highlighted entry as the root of its linkset": "Utiliser la ligne active comme root de son linkset",
    "Root #1 · {name}": "Root #1 · {name}", "#{number} · {name}": "#{number} · {name}",
    "Invalid LiveMesh grouping": "Groupement LiveMesh invalide",
    "Avatar bone weights must be finite": "Les poids des os de l’avatar doivent être finis",
    "Mesh positions must be finite": "Les positions du maillage doivent être finies",
    "Include this object in Aurora's live preview": "Inclure cet objet dans l’aperçu live Aurora",
    "Remove from synchronization": "Retirer de la synchronisation",
    "Remove this entry from Aurora's preview list; keep the Blender object":
        "Retirer cette entrée de la liste d’aperçu Aurora ; conserver l’objet Blender",
    "Missing object": "Objet manquant", "{enabled} / {total} enabled": "{enabled} / {total} actifs",
    "Add selected meshes to start syncing.": "Ajoutez la sélection pour commencer.",
    "Update Aurora Viewer to preview non-rigged meshes": "Mettez Aurora Viewer à jour pour prévisualiser les maillages non riggés",
    "Update Aurora Viewer to preview linked objects": "Mettez Aurora Viewer à jour pour prévisualiser les objets liés",
    "Select one or more rigged meshes first": "Sélectionnez d’abord un ou plusieurs maillages riggés",
    "Update now": "Mettre à jour maintenant", "Aurora port": "Port Aurora",
    "Automatic updates": "Mise à jour automatique", "Interval (seconds)": "Intervalle (secondes)",
    "Only changed content is sent.": "Envoi uniquement si modification.",
    "Manual updates only.": "Mise à jour par bouton uniquement.",
    "Send changed objects and materials at the chosen interval; disable for manual updates":
        "Envoyer les objets et matériaux modifiés à l’intervalle choisi ; désactiver pour passer en mode manuel",
    "Minimum time in seconds between automatic updates; unchanged content is not sent":
        "Délai minimum en secondes entre les mises à jour automatiques ; aucun envoi si le contenu est inchangé",
    "Send the latest enabled objects and materials immediately, even in manual mode":
        "Envoyer immédiatement les objets actifs et leurs matériaux, même en mode manuel",
    "Coordinates": "Coordonnées", "SL / Z up": "SL / Z vertical", "Blender -Y forward": "Blender -Y vers l’avant",
    "Receive avatar shape": "Recevoir la shape de l’avatar",
    "Automatic (Avastar / SL)": "Automatique (Avastar / SL)",
    "Detect each rig's rest axes; create new rigs facing Blender -Y": "Détectez les axes de repos de chaque squelette ; les nouveaux rigs regardent vers −Y comme Avastar",
    "Enable LiveMesh in Aurora first.": "Activez d’abord LiveMesh dans Aurora.",
    "Local preview · no asset upload": "Aperçu local · sans import automatique",
}.items()}}


def register():
    tracking.on_change = _request_capture
    tools_ui.on_weights_changed = _weights_changed
    bpy.app.translations.register(__name__, _translations)
    for cls in _classes:
        bpy.utils.register_class(cls)
    bpy.types.Scene.aurora_live = PointerProperty(type=AURORA_PG_live)
    bpy.types.Object.aurora_live_ik_handle_size = FloatProperty(name="Handle size", default=1, min=0.4, max=3,
                                                              update=_handle_size_changed)
    bpy.types.WindowManager.aurora_live_tab = EnumProperty(name="Section", items=tools_ui.TABS, default="LIVE", options={"SKIP_SAVE"})
    bpy.types.WindowManager.aurora_live_show_connection = BoolProperty(default=False, options={"SKIP_SAVE"})
    bpy.types.WindowManager.aurora_live_show_starter = BoolProperty(default=False, options={"SKIP_SAVE"})
    bpy.types.WindowManager.aurora_live_show_advanced_bones = BoolProperty(default=False, options={"SKIP_SAVE"})
    bpy.app.handlers.depsgraph_update_post.append(_changed)
    bpy.app.handlers.load_pre.append(_load_pre)
    bpy.app.handlers.load_post.append(_refresh_handles)
    bpy.app.timers.register(_refresh_handles, first_interval=0.1)


def unregister():
    _stop()
    tracking.on_change = None
    tools_ui.on_weights_changed = None
    bpy.app.translations.unregister(__name__)
    if bpy.app.timers.is_registered(_refresh_handles):
        bpy.app.timers.unregister(_refresh_handles)
    for handlers, callback in ((bpy.app.handlers.depsgraph_update_post, _changed), (bpy.app.handlers.load_pre, _load_pre),
                               (bpy.app.handlers.load_post, _refresh_handles)):
        if callback in handlers:
            handlers.remove(callback)
    if hasattr(bpy.types.Scene, "aurora_live"):
        del bpy.types.Scene.aurora_live
    if hasattr(bpy.types.Object, "aurora_live_ik_handle_size"):
        del bpy.types.Object.aurora_live_ik_handle_size
    for name in ("aurora_live_tab", "aurora_live_show_connection", "aurora_live_show_starter", "aurora_live_show_advanced_bones"):
        if hasattr(bpy.types.WindowManager, name):
            delattr(bpy.types.WindowManager, name)
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)
