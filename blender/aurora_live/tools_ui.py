"""Rig display, independent IK switches, and skinning tools in the Aurora sidebar."""
import bpy
from bpy.app.translations import pgettext_iface as tr
from bpy.props import BoolProperty, EnumProperty
from . import rigging, skinning

on_weights_changed = None


class AURORA_OT_bones(bpy.types.Operator):
    bl_idname = "aurora.bone_visibility"
    bl_label = "Show / hide bones"
    bl_options = {"REGISTER", "UNDO"}
    part: EnumProperty(items=[(key, label, "") for key, label in rigging.PARTS] +
                       [("ALL", "All bones", ""), ("DEFORM", "Deform bones", "")])
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


class AURORA_PT_rig(bpy.types.Panel):
    bl_label = "Skeleton / Bento / IK"
    bl_idname = "AURORA_PT_rig"
    bl_parent_id = "AURORA_PT_live"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Aurora"

    def draw(self, context):
        layout = self.layout
        layout.prop(context.scene.aurora_live, "rig")
        try:
            rig = rigging.resolve(context)
        except ValueError:
            layout.label(text="Choose an avatar rig first", icon="INFO")
            return
        row = layout.row(align=True)
        row.prop(rig, "show_in_front", text="In front")
        row.prop(rig.data, "show_names", text="Names")
        layout.prop(rig.data, "pose_position", expand=True)
        box = layout.box()
        box.label(text="Visible parts")
        row = box.row(align=True)
        for enabled, label in ((True, "Show all"), (False, "Hide all")):
            op = row.operator("aurora.bone_visibility", text=label)
            op.part, op.visible = "ALL", enabled
        for key, label in rigging.PARTS:
            count = sum(rigging.part(bone) == key for bone in rigging.bones(rig))
            if not count:
                continue
            enabled = rigging.part_visible(rig, key)
            op = box.operator("aurora.bone_visibility", text=tr(label) + " (" + str(count) + ")",
                              translate=False, icon="HIDE_OFF" if enabled else "HIDE_ON", depress=enabled)
            op.part, op.visible = key, not enabled
        box = layout.box()
        box.label(text="Hand / foot IK", icon="CON_KINEMATIC")
        if not rigging.ready(rig):
            box.operator("aurora.create_ik")
        else:
            for key, (label, *_names) in rigging.LIMBS.items():
                enabled = rigging.constraint(rig, key).influence > 0.5
                split = box.split(factor=0.60, align=True)
                op = split.operator("aurora.switch_ik", text=label, depress=enabled,
                                  icon="CHECKBOX_HLT" if enabled else "CHECKBOX_DEHLT")
                op.limb, op.enabled = key, not enabled
                row = split.row(align=True)
                for pole, text in ((False, "Target"), (True, "Pole")):
                    op = row.operator("aurora.select_ik", text=text)
                    op.limb, op.pole = key, pole
        box.label(text="IK poses stay in Blender.", icon="INFO")
        box.label(text="Aurora animates the rest mesh.")


class AURORA_PT_skin(bpy.types.Panel):
    bl_label = "Weights / transfer"
    bl_idname = "AURORA_PT_skin"
    bl_parent_id = "AURORA_PT_live"
    bl_space_type = "VIEW_3D"
    bl_region_type = "UI"
    bl_category = "Aurora"

    def draw(self, context):
        layout = self.layout
        prefs = context.scene.aurora_live
        layout.prop(prefs, "weight_scope")
        layout.operator("aurora.auto_weights", icon="MOD_ARMATURE")
        box = layout.box()
        box.label(text="Weight source")
        box.prop(prefs, "weight_source", text="")
        box.prop(prefs, "transfer_method")
        box.label(text="Max distance (0 = unlimited)")
        box.prop(prefs, "transfer_distance", text="")
        box.operator("aurora.transfer_weights", icon="MOD_DATA_TRANSFER")
        box.label(text="Select target meshes; the source stays unchanged.")
        layout.operator("aurora.transfer_weights", text="Normalize / limit to four weights", icon="BRUSH_DATA").clean = True


CLASSES = (AURORA_OT_bones, AURORA_OT_ik_setup, AURORA_OT_ik_switch, AURORA_OT_ik_select,
           AURORA_OT_transfer, AURORA_PT_rig, AURORA_PT_skin)
