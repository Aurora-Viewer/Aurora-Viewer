"""Native Blender regression coverage for Bento display, IK and skin transfer."""
from pathlib import Path
import math
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch
import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, handles, rigging, skinning, starter, tracking


class RigToolsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        aurora_live.register()

    @classmethod
    def tearDownClass(cls):
        aurora_live.unregister()

    def setUp(self):
        aurora_live._stop()
        if bpy.context.object and bpy.context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        bpy.context.scene.aurora_live.objects.clear()
        bpy.context.scene.cursor.location = (0, 0, 0)
        self.rig, self.shirt = starter.create(bpy.context)
        bpy.context.scene.aurora_live.rig = self.rig

    def mesh(self, name, points, faces=()):
        data = bpy.data.meshes.new(name)
        data.from_pydata(points, [], faces)
        obj = bpy.data.objects.new(name, data)
        bpy.context.collection.objects.link(obj)
        return obj

    def weighted_donor(self):
        obj = self.mesh("Donor", [(0, 0, 0), (1, 0, 0), (0, 1, 0)], [(0, 1, 2)])
        for index, name in enumerate(("mPelvis", "mTorso", "mChest")):
            obj.vertex_groups.new(name=name).add([index], 1, "REPLACE")
        modifier = obj.modifiers.new("Rig", "ARMATURE")
        modifier.object = self.rig
        return obj

    def values(self, obj, index=0):
        names = {g.index: g.name for g in obj.vertex_groups}
        return {names[g.group]: g.weight for g in obj.data.vertices[index].groups}

    def pose(self, key):
        return [self.rig.pose.bones[name].matrix.copy() for name in rigging.chain(self.rig, key)]

    def assertPoseNear(self, before, after, tolerance=0.002):
        for a, b in zip(before, after):
            self.assertLess((a.translation - b.translation).length, tolerance)
            angle = a.to_quaternion().rotation_difference(b.to_quaternion()).angle
            self.assertLess(min(angle, abs(2 * math.pi - angle)), 0.025)

    def test_setup_is_idempotent_and_keeps_sl_rest_bones_and_live_export(self):
        before = {b.name: b.matrix_local.copy() for b in self.rig.data.bones}
        snapshot = exporter.snapshot([self.shirt], set(before), 1)
        self.assertEqual(bpy.ops.aurora.create_ik(), {"FINISHED"})
        rigging.setup(bpy.context, self.rig)
        self.assertEqual(len(self.rig.data.bones), 167)
        for name, matrix in before.items():
            self.assertEqual(matrix, self.rig.data.bones[name].matrix_local)
        controls = [b for b in self.rig.data.bones if b.get("aurora_control")]
        self.assertEqual(len(controls), 8)
        self.assertTrue(all(not b.use_deform for b in controls))
        for key in rigging.LIMBS:
            rigging.set_ik(bpy.context, self.rig, key, True)
        self.assertEqual(snapshot, exporter.snapshot([self.shirt], set(before), 1))

    def test_visibility_is_independent_for_bento_body_and_controls(self):
        rigging.setup(bpy.context, self.rig)
        rigging.show(self.rig, "ALL", False)
        for key in ("HANDS", "ARMS", "FACE", "FITTED", "IK", "LEGS", "TAIL", "WINGS", "HIND"):
            with self.subTest(part=key):
                rigging.show(self.rig, key, True)
                self.assertTrue(rigging.part_visible(self.rig, key))
                self.assertFalse(rigging.part_visible(self.rig, "BODY"))
                rigging.show(self.rig, key, False)
                self.assertFalse(rigging.part_visible(self.rig, key))
        rigging.show(self.rig, "ALL", True)
        self.assertTrue(all(rigging.visible(self.rig, b) for b in self.rig.data.bones))

    def test_classic_back_and_head_are_separate_from_the_folded_bento_spine(self):
        names = lambda key: {b.name for b in self.rig.data.bones if rigging.part(b) == key}
        self.assertEqual(names("BACK"), {"mPelvis", "mTorso", "mChest"})
        self.assertEqual(names("HEAD"), {"mNeck", "mHead", "mSkull"})
        self.assertEqual(names("SPINE"), {"mSpine1", "mSpine2", "mSpine3", "mSpine4"})
        self.assertTrue(rigging.part_visible(self.rig, "BACK"))
        self.assertFalse(rigging.part_visible(self.rig, "SPINE"))
        self.assertFalse(bpy.context.window_manager.aurora_live_show_advanced_bones)
        # The XML doubles back over the same segments, rather than dividing
        # each classic segment into three shorter anatomical vertebrae.
        for original, reverse, forward in (("mPelvis", "mSpine1", "mSpine2"), ("mTorso", "mSpine3", "mSpine4")):
            a, b, c = (self.rig.data.bones[name] for name in (original, reverse, forward))
            self.assertLess((a.head_local - b.tail_local).length, 1e-6)
            self.assertLess((a.tail_local - b.head_local).length, 1e-6)
            self.assertLess((a.head_local - c.head_local).length, 1e-6)
            self.assertEqual(c.parent, b)

    def test_back_resets_existing_full_display_in_object_pose_and_edit_modes(self):
        for obj in bpy.context.selected_objects:
            obj.select_set(False)
        self.rig.select_set(True)
        bpy.context.view_layer.objects.active = self.rig
        for mode in ("OBJECT", "POSE", "EDIT"):
            bpy.ops.object.mode_set(mode=mode)
            with self.subTest(mode=mode):
                bpy.ops.aurora.bone_visibility(part="ALL", visible=True)
                bpy.ops.aurora.bone_visibility(part="BACK", visible=True)
                for bone in rigging.bones(self.rig):
                    self.assertEqual(rigging.visible(self.rig, bone), rigging.part(bone) != "SPINE", bone.name)
                bpy.ops.aurora.bone_visibility(part="SPINE", visible=True)
                bpy.ops.aurora.bone_visibility(part="BACK", visible=False)
                for bone in rigging.bones(self.rig):
                    if mode == "EDIT":
                        native = bone
                    else:
                        pose_bone = self.rig.pose.bones[bone.name]
                        native = pose_bone if hasattr(pose_bone, "hide") else bone
                    self.assertEqual(native.hide, rigging.part(bone) in {"BACK", "SPINE"}, bone.name)
                self.assertEqual(self.rig.mode, mode)
        bpy.ops.object.mode_set(mode="OBJECT")

    def test_advanced_spine_stays_accessible_with_aliases_and_legacy_body_calls(self):
        self.rig.data.bones["mSpine2"].name = "Custom lumbar joint"
        self.assertEqual(rigging.part(self.rig.data.bones["Custom lumbar joint"]), "SPINE")
        for full in ("ALL", "DEFORM"):
            rigging.show(self.rig, "ALL", False)
            rigging.show(self.rig, full, True)
            self.assertTrue(all(rigging.visible(self.rig, b) for b in self.rig.data.bones))
            rigging.show(self.rig, "BACK", True)
            self.assertFalse(rigging.part_visible(self.rig, "SPINE"))
        rigging.show(self.rig, "ALL", False)
        bpy.ops.aurora.bone_visibility(part="BODY", visible=True)
        self.assertEqual(sum(rigging.visible(self.rig, b) for b in self.rig.data.bones), 6)
        self.assertTrue(rigging.part_visible(self.rig, "HEAD"))
        bpy.ops.aurora.bone_visibility(part="SPINE", visible=True)
        self.assertEqual(sum(rigging.visible(self.rig, b) for b in self.rig.data.bones), 10)
        bpy.ops.aurora.bone_visibility(part="BACK", visible=True)
        self.assertFalse(rigging.part_visible(self.rig, "SPINE"))

    def test_simple_back_preserves_spine_weights_animation_deformation_and_export(self):
        self.shirt.vertex_groups.clear()
        for name in sorted(rigging.SPINE):
            self.shirt.vertex_groups.new(name=name).add(list(range(len(self.shirt.data.vertices))), 0.25, "REPLACE")
        donor = self.shirt.copy()
        donor.data = self.shirt.data.copy()
        bpy.context.collection.objects.link(donor)
        donor.select_set(False)
        # Transfer and cleanup must retain hidden spine influences as well.
        skinning.apply(bpy.context, [self.shirt], self.rig, donor)
        skinning.apply(bpy.context, [self.shirt], self.rig, clean=True)
        self.assertEqual(set(self.values(self.shirt)), rigging.SPINE)
        joint = self.rig.pose.bones["mSpine2"]
        joint.rotation_mode = "XYZ"
        joint.rotation_euler.x = 0.3
        joint.keyframe_insert(data_path="rotation_euler", frame=1)
        bpy.context.view_layer.update()
        rest = {b.name: (b.matrix_local.copy(), b.parent.name if b.parent else None, b.use_deform) for b in self.rig.data.bones}
        weights = [self.values(self.shirt, i) for i in range(len(self.shirt.data.vertices))]
        action = self.rig.animation_data.action
        supported = set(rest)
        snapshot = exporter.snapshot([self.shirt], supported, 1)
        def evaluated():
            obj = self.shirt.evaluated_get(bpy.context.evaluated_depsgraph_get())
            return [v.co.copy() for v in obj.data.vertices]
        posed = evaluated()
        selected = list(bpy.context.selected_objects)
        for key, enabled in (("ALL", True), ("BACK", True), ("SPINE", True), ("BACK", False), ("BACK", True)):
            bpy.ops.aurora.bone_visibility(part=key, visible=enabled)
        bpy.context.view_layer.update()
        self.assertEqual(rest, {b.name: (b.matrix_local.copy(), b.parent.name if b.parent else None, b.use_deform) for b in self.rig.data.bones})
        self.assertEqual(weights, [self.values(self.shirt, i) for i in range(len(self.shirt.data.vertices))])
        self.assertEqual(action, self.rig.animation_data.action)
        self.assertAlmostEqual(joint.rotation_euler.x, 0.3)
        self.assertEqual(snapshot, exporter.snapshot([self.shirt], supported, 1))
        self.assertTrue(all((a - b).length < 1e-7 for a, b in zip(posed, evaluated())))
        self.assertEqual(selected, list(bpy.context.selected_objects))
        self.assertEqual(self.shirt.find_armature(), self.rig)

    def test_each_ik_switch_preserves_fk_and_is_independent(self):
        rigging.setup(bpy.context, self.rig)
        for key in rigging.LIMBS:
            with self.subTest(limb=key):
                before = self.pose(key)
                rigging.set_ik(bpy.context, self.rig, key, True)
                self.assertPoseNear(before, self.pose(key))
                self.assertEqual([rigging.constraint(self.rig, k).influence for k in rigging.LIMBS],
                                 [float(k == key) for k in rigging.LIMBS])
                target = self.rig.pose.bones[rigging.control_name(key)]
                matrix = target.matrix.copy()
                root = self.rig.pose.bones[rigging.chain(self.rig, key)[0]]
                matrix.translation = root.head + (matrix.translation - root.head) * 0.85 + Vector((-0.03, 0, 0))
                target.matrix = matrix
                bpy.context.view_layer.update()
                middle = self.rig.pose.bones[rigging.chain(self.rig, key)[1]]
                self.assertLess((middle.tail - matrix.translation).length, 0.003)
                solved = self.pose(key)
                rigging.set_ik(bpy.context, self.rig, key, False)
                self.assertPoseNear(solved, self.pose(key), 0.0001)

    def test_widgets_have_distinct_outlines_and_preserve_solver_pose_and_export(self):
        rigging.setup(bpy.context, self.rig)
        for key in rigging.LIMBS:
            rigging.set_ik(bpy.context, self.rig, key, True)
        before = {b.name: b.matrix.copy() for b in self.rig.pose.bones}
        rest = {b.name: (b.matrix_local.copy(), b.parent.name if b.parent else None) for b in self.rig.data.bones}
        snapshot = exporter.snapshot([self.shirt], set(rest), 1)
        constraints = {key: (rigging.constraint(self.rig, key).influence, rigging.constraint(self.rig, key).pole_angle) for key in rigging.LIMBS}
        self.assertEqual(bpy.ops.aurora.refresh_ik_handles(), {"FINISHED"})
        bpy.context.view_layer.update()
        styles = set()
        for key in rigging.LIMBS:
            for pole in (False, True):
                bone = self.rig.pose.bones[rigging.control_name(key, pole)]
                self.assertGreater(len(bone.custom_shape.data.edges), 24)
                self.assertFalse(bone.custom_shape.users_collection)
                self.assertFalse(bone.use_custom_shape_bone_size)
                styles.add(bone.custom_shape["aurora_ik_shape"].split(":")[0][:-1])
                self.assertFalse(bone.bone.use_deform)
        self.assertEqual(styles, {"HAND", "FOOT", "ELBOW", "KNEE"})
        self.assertEqual(before, {b.name: b.matrix.copy() for b in self.rig.pose.bones})
        self.assertEqual(rest, {b.name: (b.matrix_local.copy(), b.parent.name if b.parent else None) for b in self.rig.data.bones})
        self.assertEqual(snapshot, exporter.snapshot([self.shirt], set(rest), 1))
        self.assertEqual(constraints, {key: (rigging.constraint(self.rig, key).influence, rigging.constraint(self.rig, key).pole_angle) for key in rigging.LIMBS})

    def test_widget_size_is_per_rig_and_cache_does_not_create_scene_meshes(self):
        rigging.setup(bpy.context, self.rig)
        names = [rigging.control_name(key, pole) for key in rigging.LIMBS for pole in (False, True)]
        scales = {name: self.rig.pose.bones[name].custom_shape_scale_xyz.x if hasattr(self.rig.pose.bones[name], "custom_shape_scale_xyz") else self.rig.pose.bones[name].custom_shape_scale for name in names}
        other = self.rig.copy()
        bpy.context.collection.objects.link(other)
        objects, meshes, scene_objects = len(bpy.data.objects), len(bpy.data.meshes), len(bpy.context.scene.objects)
        self.rig.aurora_live_ik_handle_size = 1.75
        for _ in range(3):
            handles.apply(self.rig)
        for name in names:
            pose = self.rig.pose.bones[name]
            other_pose = other.pose.bones[name]
            scale = pose.custom_shape_scale_xyz.x if hasattr(pose, "custom_shape_scale_xyz") else pose.custom_shape_scale
            other_scale = other_pose.custom_shape_scale_xyz.x if hasattr(other_pose, "custom_shape_scale_xyz") else other_pose.custom_shape_scale
            self.assertAlmostEqual(scale, scales[name] * 1.75)
            self.assertAlmostEqual(other_scale, scales[name])
        self.assertEqual((objects, meshes, scene_objects), (len(bpy.data.objects), len(bpy.data.meshes), len(bpy.context.scene.objects)))

    def test_old_rigs_migrate_without_replacing_authored_widgets_or_altering_the_pose(self):
        rigging.setup(bpy.context, self.rig)
        names = [rigging.control_name(key, pole) for key in rigging.LIMBS for pole in (False, True)]
        for name in names:
            self.rig.pose.bones[name].custom_shape = None
        authored = bpy.data.objects.new("Authored hand widget", bpy.data.meshes.new("Authored widget"))
        self.rig.pose.bones[names[0]].custom_shape = authored
        before = {bone.name: bone.matrix.copy() for bone in self.rig.pose.bones}
        aurora_live._refresh_handles()
        self.assertEqual(self.rig.pose.bones[names[0]].custom_shape, authored)
        self.assertTrue(all(self.rig.pose.bones[name].custom_shape for name in names[1:]))
        self.assertEqual(before, {bone.name: bone.matrix.copy() for bone in self.rig.pose.bones})
        handles.apply(self.rig, replace=True)
        self.assertIsNot(self.rig.pose.bones[names[0]].custom_shape, authored)

    def test_visibility_buttons_change_the_native_pose_hide_flag_repeatedly(self):
        rigging.setup(bpy.context, self.rig)
        for obj in bpy.context.selected_objects:
            obj.select_set(False)
        self.rig.select_set(True)
        bpy.context.view_layer.objects.active = self.rig
        for mode in ("OBJECT", "POSE"):
            bpy.ops.object.mode_set(mode=mode)
            for key, _label in rigging.PARTS:
                with self.subTest(mode=mode, part=key):
                    members = [b for b in self.rig.data.bones if rigging.part(b) == key]
                    for _repeat in range(2):
                        for visible in (True, False):
                            self.assertEqual(bpy.ops.aurora.bone_visibility(part=key, visible=visible), {"FINISHED"})
                            for bone in members:
                                pose_bone = self.rig.pose.bones[bone.name]
                                native = pose_bone if hasattr(pose_bone, "hide") else bone
                                self.assertEqual(native.hide, not visible, bone.name)
                                self.assertEqual(rigging.visible(self.rig, bone), visible, bone.name)

    def test_switch_from_bent_fk_pose_and_hidden_control_selection(self):
        rigging.setup(bpy.context, self.rig)
        for key in rigging.LIMBS:
            root, middle, end = (self.rig.pose.bones[name] for name in rigging.chain(self.rig, key))
            middle.rotation_mode = "XYZ"
            middle.rotation_euler.x = 0.5
            bpy.context.view_layer.update()
            before = self.pose(key)
            rigging.set_ik(bpy.context, self.rig, key, True)
            self.assertPoseNear(before, self.pose(key))
            rigging.show(self.rig, "IK", False)
            rigging.select_control(bpy.context, self.rig, key, True)
            self.assertEqual(self.rig.mode, "POSE")
            self.assertEqual(bpy.context.active_pose_bone.name, rigging.control_name(key, True))
            bpy.ops.object.mode_set(mode="OBJECT")

    def test_existing_foreign_constraints_are_rejected_before_changes(self):
        self.rig.pose.bones["mElbowLeft"].constraints.new("LIMIT_ROTATION")
        with self.assertRaisesRegex(ValueError, "already has constraints"):
            rigging.setup(bpy.context, self.rig)
        self.assertEqual(len(self.rig.data.bones), 159)

    def test_visibility_in_edit_mode_and_rest_pose_rejects_ik_switch(self):
        rigging.setup(bpy.context, self.rig)
        bpy.context.view_layer.objects.active = self.rig
        self.shirt.select_set(False)
        self.rig.select_set(True)
        bpy.ops.object.mode_set(mode="EDIT")
        rigging.show(self.rig, "ALL", False)
        rigging.show(self.rig, "FACE", True)
        self.assertTrue(rigging.part_visible(self.rig, "FACE"))
        self.assertFalse(rigging.part_visible(self.rig, "ARMS"))
        bpy.ops.object.mode_set(mode="OBJECT")
        self.rig.data.pose_position = "REST"
        with self.assertRaisesRegex(ValueError, "Pose Position"):
            rigging.set_ik(bpy.context, self.rig, "HAND_L", True)

    def test_shared_armature_data_is_copied_for_control_creation(self):
        other = self.rig.copy()
        bpy.context.collection.objects.link(other)
        rigging.setup(bpy.context, self.rig)
        self.assertIsNot(self.rig.data, other.data)
        self.assertEqual(len(other.data.bones), 159)

    def test_surface_transfer_interpolates_and_preserves_masks_uv_shapes_and_donor(self):
        donor = self.weighted_donor()
        target = self.mesh("Target", [(0.25, 0.25, 0)])
        target.vertex_groups.new(name="Cloth mask").add([0], 0.37, "REPLACE")
        target.data.uv_layers.new(name="Cloth UV")
        target.shape_key_add(name="Basis")
        target.shape_key_add(name="Fit")
        before = [self.values(donor, i) for i in range(3)]
        counts = len(bpy.data.objects), len(bpy.data.meshes)
        skinning.apply(bpy.context, [target, donor], self.rig, donor)
        self.assertEqual(counts, (len(bpy.data.objects), len(bpy.data.meshes)))
        for name, weight in (("mPelvis", 0.5), ("mTorso", 0.25), ("mChest", 0.25), ("Cloth mask", 0.37)):
            self.assertAlmostEqual(self.values(target)[name], weight)
        self.assertEqual(before, [self.values(donor, i) for i in range(3)])
        self.assertEqual(target.data.uv_layers[0].name, "Cloth UV")
        self.assertEqual(len(target.data.shape_keys.key_blocks), 2)
        self.assertEqual(target.find_armature(), self.rig)

    def test_nearest_vertex_uses_world_transforms_and_unposed_donor(self):
        donor = self.weighted_donor()
        donor.location = (2, 3, 1)
        donor.scale = (2, 2, 2)
        self.rig.pose.bones["mPelvis"].location.x = 5
        target = self.mesh("Target", [(0, 0, 0)])
        target.location = (3.9, 3.1, 1)
        bpy.context.view_layer.update()
        before = target.matrix_world.copy()
        skinning.apply(bpy.context, [target], self.rig, donor, "VERTEX")
        self.assertEqual(self.values(target), {"mTorso": 1})
        self.assertEqual(target.matrix_world, before)

    def test_distance_and_missing_weights_fail_atomically_for_all_targets(self):
        donor = self.weighted_donor()
        near = self.mesh("Near", [(0.2, 0.2, 0)])
        far = self.mesh("Far", [(20, 20, 20)])
        near.vertex_groups.new(name="mPelvis").add([0], 0.7, "REPLACE")
        for failure in ("distance", "unweighted"):
            with self.subTest(failure=failure):
                if failure == "unweighted":
                    donor.vertex_groups.clear()
                with self.assertRaisesRegex(ValueError, "original weights were kept"):
                    skinning.apply(bpy.context, [near, far], self.rig, donor, max_distance=1 if failure == "distance" else 0)
                self.assertAlmostEqual(self.values(near)["mPelvis"], 0.7)
                self.assertIsNone(near.find_armature())

    def test_transfer_maps_canonical_aliases_and_includes_fitted_bento_weights(self):
        donor = self.weighted_donor()
        donor.vertex_groups.clear()
        self.rig.data.bones["mPelvis"].name = "Donor Hip"
        for name in ("Donor Hip", "BELLY", "mHandIndex1Left"):
            donor.vertex_groups.new(name=name).add([0, 1, 2], 1 / 3, "REPLACE")
        target_rig = self.rig.copy()
        target_rig.data = self.rig.data.copy()
        bpy.context.collection.objects.link(target_rig)
        target_rig.data.bones["Donor Hip"].name = "Target Hip"
        target = self.mesh("Target", [(0.2, 0.2, 0)])
        skinning.apply(bpy.context, [target], target_rig, donor)
        self.assertEqual(set(self.values(target)), {"Target Hip", "BELLY", "mHandIndex1Left"})
        self.assertAlmostEqual(sum(self.values(target).values()), 1)

    def test_transfer_evaluates_donor_shape_keys_and_mirror_without_mutation(self):
        donor = self.weighted_donor()
        donor.shape_key_add(name="Basis")
        shape = donor.shape_key_add(name="Raised")
        for point in shape.data:
            point.co.z = 1
        shape.value = 1
        donor.modifiers.new("Symmetry", "MIRROR")
        target = self.mesh("Target", [(-0.25, 0.25, 1)])
        skinning.apply(bpy.context, [target], self.rig, donor, max_distance=0.01)
        self.assertAlmostEqual(self.values(target)["mPelvis"], 0.5)
        self.assertAlmostEqual(self.values(target)["mTorso"], 0.25)
        self.assertEqual(len(donor.data.vertices), 3)
        self.assertEqual(shape.value, 1)
        self.assertEqual(len(donor.modifiers), 2)

    def test_clean_normalizes_four_influences_including_fitted_and_bento(self):
        target = self.mesh("Target", [(0, 0, 0)])
        names = ("mPelvis", "mTorso", "mChest", "mHandIndex1Left", "BELLY", "mHead")
        for index, name in enumerate(names):
            target.vertex_groups.new(name=name).add([0], (index + 1) * 0.1, "REPLACE")
        skinning.apply(bpy.context, [target], self.rig, clean=True)
        result = self.values(target)
        self.assertEqual(set(result), set(names[2:]))
        self.assertAlmostEqual(sum(result.values()), 1)

    def test_transfer_matches_the_current_target_shape_without_changing_vertex_indices(self):
        donor = self.weighted_donor()
        target = self.mesh("Target", [(0, 0, 10)])
        target.shape_key_add(name="Basis")
        shape = target.shape_key_add(name="Fit")
        shape.data[0].co = (0.25, 0.25, 0)
        shape.value = 1
        skinning.apply(bpy.context, [target], self.rig, donor, max_distance=0.01)
        self.assertAlmostEqual(self.values(target)["mPelvis"], 0.5)
        self.assertEqual(target.data.vertices[0].co.z, 10)
        self.assertEqual(shape.value, 1)

    def test_locked_target_groups_reject_without_modifying_weights(self):
        donor = self.weighted_donor()
        target = self.mesh("Target", [(0.2, 0.2, 0)])
        group = target.vertex_groups.new(name="mPelvis")
        group.add([0], 0.42, "REPLACE")
        group.lock_weight = True
        with self.assertRaisesRegex(ValueError, "Unlock"):
            skinning.apply(bpy.context, [target], self.rig, donor)
        self.assertAlmostEqual(group.weight(0), 0.42)

    def test_shared_target_data_and_weight_paint_selection_are_preserved(self):
        donor = self.weighted_donor()
        other = self.shirt.copy()
        bpy.context.collection.objects.link(other)
        other.select_set(False)
        old_data = other.data
        before = self.values(other)
        prefs = bpy.context.scene.aurora_live
        prefs.weight_source = donor
        bpy.ops.object.mode_set(mode="WEIGHT_PAINT")
        self.assertEqual(bpy.ops.aurora.transfer_weights(), {"FINISHED"})
        self.assertEqual(self.shirt.mode, "WEIGHT_PAINT")
        self.assertIsNot(self.shirt.data, old_data)
        self.assertEqual(other.data, old_data)
        self.assertEqual(self.values(other), before)

    def test_transfer_operator_invalidates_live_preview_without_resuming_paused_entries(self):
        donor = self.weighted_donor()
        prefs = bpy.context.scene.aurora_live
        prefs.weight_source = donor
        tracking.add(bpy.context.scene, [self.shirt])
        prefs.objects[0].enabled = False
        fake = SimpleNamespace(close=lambda: None)
        with patch.object(aurora_live, "_client", fake):
            aurora_live._dirty = False
            self.assertEqual(bpy.ops.aurora.transfer_weights(), {"FINISHED"})
            self.assertTrue(aurora_live._dirty)
            self.assertFalse(prefs.objects[0].enabled)


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(RigToolsTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_RIG_TOOLS_TESTS_OK", flush=True)
