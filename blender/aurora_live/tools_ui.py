"""Rig display, independent IK switches, and skinning tools in the Aurora sidebar."""
import bpy
from bpy.app.translations import pgettext_iface as tr
from bpy.props import BoolProperty, EnumProperty
from . import rigging, skinning, posing, handles

on_weights_changed = None

TABS = (
    ("LIVE", "Live", "Connect and manage synchronized meshes", "LINKED", 0),
    ("RIG", "Skeleton", "Create an SL avatar and show or hide bone parts", "ARMATURE_DATA", 1),
    ("IK", "Controls", "Hand/foot IK, root/COG and eye targets", "CON_KINEMATIC", 2),
    ("WEIGHTS", "Weights", "Calculate, transfer and normalize mesh weights", "WPAINT_HLT", 3),
)


def tab(context):
    return getattr(context.window_manager, "aurora_live_tab", "LIVE")


def foldout(layout, manager, property_name, title):
    opened = getattr(manager, property_name)
    layout.prop(manager, property_name, text=title, icon="TRIA_DOWN" if opened else "TRIA_RIGHT", emboss=False)
    return layout.box() if opened else None


class AURORA_OT_bones(bpy.types.Operator):
    bl_idname = "aurora.bone_visibility"
    bl_label = "Show / hide bones"
    bl_description = "Show or hide bones without changing the rig or its weights"
    bl_options = {"REGISTER", "UNDO"}
    # Stable values retain old redo/operator presets when categories change.
    part: EnumProperty(items=[(key, label, "", "NONE", number) for key, label, number in (
        ("BACK", "Back / torso", 13), ("HEAD", "Head / neck", 14),
        ("BODY", "Body", 0), ("ARMS", "Arms", 1), ("LEGS", "Legs", 2), ("HANDS", "Bento fingers", 3),
        ("FACE", "Bento face / eyes", 4), ("WINGS", "Bento wings", 5), ("TAIL", "Bento tail", 6),
        ("HIND", "Bento hind limbs", 7), ("FITTED", "Fitted Mesh volumes", 8), ("IK", "IK controls", 9),
        ("EXTRA", "Other bones", 10), ("ALL", "All bones", 11), ("DEFORM", "Deform bones", 12),
        ("SPINE", "Bento spine", 15), ("ROOT", "Root / COG controls", 16), ("LOOK", "Look-at targets", 17),
    )])
    visible: BoolProperty(default=True)

    def execute(self, context):
        try:
            rigging.show(rigging.resolve(context), self.part, self.visible)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_ik_setup(bpy.types.Operator):
    bl_idname = "aurora.create_ik"
    bl_label = "Create hand / foot IK controls"
    bl_description = "Add eight non-deforming target and pole bones; keep SL rest bones unchanged"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        try:
            rigging.setup(context, rigging.resolve(context))
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_ik_handles(bpy.types.Operator):
    bl_idname = "aurora.refresh_ik_handles"
    bl_label = "Refresh IK handles"
    bl_description = "Replace IK widgets without changing the pose, weights or solver"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        try:
            rig = rigging.resolve(context)
            rigging.editable(rig)
            if rig.mode == "EDIT":
                raise ValueError("Leave Edit Mode before creating IK controls")
            if not rigging.ready(rig):
                raise ValueError("Create Aurora IK controls first")
            handles.apply(rig, replace=True)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_ik_poles(bpy.types.Operator):
    bl_idname = "aurora.reposition_ik_poles"
    bl_label = "Reposition elbow / knee poles"
    bl_description = "Align poles with the current limb pose without rebuilding the rig"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        try:
            rigging.reposition_poles(context, rigging.resolve(context))
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_knee_bend(bpy.types.Operator):
    bl_idname = "aurora.correct_knee_bend"
    bl_label = "Correct knee bend"
    bl_description = "Make active knees bend toward their poles while keeping the foot targets fixed"
    bl_options = {"REGISTER", "UNDO"}

    def execute(self, context):
        try:
            rigging.correct_knees(context, rigging.resolve(context))
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_ik_switch(bpy.types.Operator):
    bl_idname = "aurora.switch_ik"
    bl_label = "Enable / disable IK"
    bl_description = "Snap IK controls to the current pose when enabling; keep the visible pose when disabling"
    bl_options = {"REGISTER", "UNDO"}
    limb: EnumProperty(items=[(key, value[0], "") for key, value in rigging.LIMBS.items()])
    enabled: BoolProperty(default=True)

    def execute(self, context):
        try:
            rigging.set_ik(context, rigging.resolve(context), self.limb, self.enabled)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_ik_select(bpy.types.Operator):
    bl_idname = "aurora.select_ik"
    bl_label = "Select IK control"
    bl_options = {"REGISTER", "UNDO"}
    limb: EnumProperty(items=[(key, value[0], "") for key, value in rigging.LIMBS.items()])
    pole: BoolProperty(default=False)

    def execute(self, context):
        try:
            rigging.select_control(context, rigging.resolve(context), self.limb, self.pole)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_pose_setup(bpy.types.Operator):
    bl_idname = "aurora.create_pose_controls"
    bl_label = "Create pose controls"
    bl_description = "Add non-deforming root/COG controls or an eye target without changing SL rest joints"
    bl_options = {"REGISTER", "UNDO"}
    kind: EnumProperty(items=[("ROOT", "Root / COG", "")] + [(k, v[0], "") for k, v in posing.EYES.items()])

    def execute(self, context):
        try:
            rig = rigging.resolve(context)
            if self.kind == "ROOT":
                posing.setup_root(context, rig)
            else:
                posing.setup_look(context, rig, self.kind)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_pose_select(bpy.types.Operator):
    bl_idname = "aurora.select_pose_control"
    bl_label = "Select pose control"
    bl_options = {"REGISTER", "UNDO"}
    kind: EnumProperty(items=[("ROOT", "Root", ""), ("COG", "COG", "")] + [(k, v[0], "") for k, v in posing.EYES.items()])

    def execute(self, context):
        try:
            name = {"ROOT": posing.ROOT, "COG": posing.COG}.get(self.kind, posing.target(self.kind))
            rigging.select_named_control(context, rigging.resolve(context), name, "ROOT" if self.kind in {"ROOT", "COG"} else "LOOK")
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_look_switch(bpy.types.Operator):
    bl_idname = "aurora.switch_look_at"
    bl_label = "Enable / disable eye focus"
    bl_description = "Follow the eye target; disabling keeps the current eye pose for manual rotation"
    bl_options = {"REGISTER", "UNDO"}
    kind: EnumProperty(items=[(k, v[0], "") for k, v in posing.EYES.items()])
    enabled: BoolProperty(default=True)

    def execute(self, context):
        try:
            posing.set_look(context, rigging.resolve(context), self.kind, self.enabled)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_look_center(bpy.types.Operator):
    bl_idname = "aurora.center_look_at"
    bl_label = "Center eye target"
    bl_description = "Place the eye target ahead of the current head pose"
    bl_options = {"REGISTER", "UNDO"}
    kind: EnumProperty(items=[(k, v[0], "") for k, v in posing.EYES.items()])

    def execute(self, context):
        try:
            posing.reset_look(context, rigging.resolve(context), self.kind)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        return {"FINISHED"}


class AURORA_OT_transfer(bpy.types.Operator):
    bl_idname = "aurora.transfer_weights"
    bl_label = "Transfer weights from source"
    bl_description = "Replace selected target bone weights from the source mesh, normalized to four influences"
    bl_options = {"REGISTER", "UNDO"}
    clean: BoolProperty(default=False, options={"HIDDEN"})

    @classmethod
    def poll(cls, context):
        return context.mode in {"OBJECT", "PAINT_WEIGHT"} and any(obj.type == "MESH" for obj in context.selected_objects)

    def execute(self, context):
        prefs = context.scene.aurora_live
        try:
            objects = skinning.apply(context, [obj for obj in context.selected_objects if obj.type == "MESH"],
                                     prefs.rig, None if self.clean else prefs.weight_source,
                                     prefs.transfer_method, prefs.transfer_distance, self.clean)
        except (ValueError, RuntimeError) as error:
            self.report({"ERROR"}, tr(str(error)))
            return {"CANCELLED"}
        if on_weights_changed:
            on_weights_changed(context, objects)
        self.report({"INFO"}, tr("Weights normalized to four influences" if self.clean else "Weights transferred"))
        return {"FINISHED"}


def draw_rig(layout, context):
    try:
        rig = rigging.resolve(context)
    except ValueError:
        layout.label(text="Choose an avatar rig first", icon="INFO")
        return
    row = layout.row(align=True)
    row.prop(rig, "show_in_front", text="In front")
    row.prop(rig.data, "show_names", text="Names")
    layout.prop(rig.data, "pose_position", expand=True)
    row = layout.row(align=True)
    for enabled, label in ((True, "Show all"), (False, "Hide all")):
        op = row.operator("aurora.bone_visibility", text=label)
        op.part, op.visible = "ALL", enabled
    for key, label in rigging.PARTS:
        if key == "SPINE":
            continue
        count = sum(rigging.part(bone) == key for bone in rigging.bones(rig))
        if not count:
            continue
        enabled = rigging.part_visible(rig, key)
        op = layout.operator("aurora.bone_visibility", text=tr(label) + " (" + str(count) + ")",
                          translate=False, icon="HIDE_OFF" if enabled else "HIDE_ON", depress=enabled)
        op.part, op.visible = key, not enabled
    advanced = foldout(layout, context.window_manager, "aurora_live_show_advanced_bones", "Advanced bones")
    if advanced:
        enabled = rigging.part_visible(rig, "SPINE")
        count = sum(rigging.part(bone) == "SPINE" for bone in rigging.bones(rig))
        if count:
            op = advanced.operator("aurora.bone_visibility", text=tr("Bento spine") + " (" + str(count) + ")",
                                   translate=False, icon="HIDE_OFF" if enabled else "HIDE_ON", depress=enabled)
            op.part, op.visible = "SPINE", not enabled
            advanced.label(text="Folded SL spine joints", icon="INFO")
        else:
            advanced.label(text="No Bento spine bones in this rig")


def draw_ik(layout, context):
    try:
        rig = rigging.resolve(context)
    except ValueError:
        layout.label(text="Choose an avatar rig first", icon="INFO")
        return
    if not rigging.ready(rig):
        layout.operator("aurora.create_ik", text="Create IK controls", icon="ADD")
    else:
        box = layout.box()
        box.prop(rig, "aurora_live_ik_handle_size", text="Handle size", slider=True)
        row = box.row(align=True)
        row.prop(rig, "show_in_front", text="In front")
        row.operator("aurora.refresh_ik_handles", text="", icon="FILE_REFRESH")
        box.operator("aurora.reposition_ik_poles", text="Reposition poles", icon="CON_KINEMATIC")
        for prefix, label in (("HAND", "Hands"), ("FOOT", "Feet")):
            box = layout.box()
            box.label(text=label)
            if prefix == "FOOT":
                row = box.row()
                row.enabled = any(rigging.constraint(rig, "FOOT_" + side).influence > 0.5 for side in ("L", "R"))
                row.operator("aurora.correct_knee_bend", icon="CON_KINEMATIC")
            for side, label in (("L", "Left"), ("R", "Right")):
                key = prefix + "_" + side
                enabled = rigging.constraint(rig, key).influence > 0.5
                split = box.split(factor=0.50, align=True)
                op = split.operator("aurora.switch_ik", text=label, depress=enabled,
                                    icon="CHECKBOX_HLT" if enabled else "CHECKBOX_DEHLT")
                op.limb, op.enabled = key, not enabled
                row = split.row(align=True)
                for pole, text in ((False, "Target"), (True, "Pole")):
                    op = row.operator("aurora.select_ik", text=text)
                    op.limb, op.pole = key, pole
    box = layout.box()
    box.label(text="Root / COG")
    if not posing.root_ready(rig):
        box.operator("aurora.create_pose_controls", text="Add root / COG", icon="ADD").kind = "ROOT"
    else:
        row = box.row(align=True)
        row.operator("aurora.select_pose_control", text="Root").kind = "ROOT"
        row.operator("aurora.select_pose_control", text="COG").kind = "COG"
    box = layout.box()
    box.label(text="Look at")
    for key, (label, *_names) in posing.EYES.items():
        try:
            posing.joints(rig, posing.EYES[key][1:])
        except ValueError:
            continue
        if not posing.look_ready(rig, key):
            box.operator("aurora.create_pose_controls", text=tr(label) + " +", translate=False, icon="ADD").kind = key
        else:
            row = box.row(align=True)
            enabled = posing.look_enabled(rig, key)
            op = row.operator("aurora.switch_look_at", text="Classic" if key == "CLASSIC" else "Bento", depress=enabled,
                              icon="CHECKBOX_HLT" if enabled else "CHECKBOX_DEHLT")
            op.kind, op.enabled = key, not enabled
            row.operator("aurora.select_pose_control", text="Target").kind = key
            row.operator("aurora.center_look_at", text="", icon="LOOP_BACK").kind = key
    layout.label(text="Control poses stay in Blender.", icon="INFO")
    layout.label(text="Aurora animates the rest mesh.")


def draw_automatic(layout, context):
    prefs = context.scene.aurora_live
    layout.prop(prefs, "weight_scope", text="")
    layout.operator("aurora.auto_weights", text="Recalculate weights", icon="MOD_ARMATURE")


def draw_transfer(layout, context):
    prefs = context.scene.aurora_live
    layout.label(text="Weight source")
    layout.prop(prefs, "weight_source", text="")
    layout.label(text="Weight mapping")
    layout.prop(prefs, "transfer_method", text="")
    layout.label(text="Max distance (0 = unlimited)")
    layout.prop(prefs, "transfer_distance", text="")
    layout.operator("aurora.transfer_weights", text="Transfer weights", icon="MOD_DATA_TRANSFER")
    layout.label(text="Select the target meshes.", icon="INFO")
    layout.label(text="The source stays unchanged.")


def draw_clean(layout, context):
    layout.operator("aurora.transfer_weights", text="Normalize to 4 weights", icon="BRUSH_DATA").clean = True


CLASSES = (AURORA_OT_bones, AURORA_OT_ik_setup, AURORA_OT_ik_handles, AURORA_OT_ik_poles, AURORA_OT_knee_bend, AURORA_OT_ik_switch, AURORA_OT_ik_select,
           AURORA_OT_pose_setup, AURORA_OT_pose_select, AURORA_OT_look_switch, AURORA_OT_look_center, AURORA_OT_transfer)
