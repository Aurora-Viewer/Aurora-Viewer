"""Native Blender root/COG and classic/Bento gaze integration regressions."""
from pathlib import Path
import sys
import unittest
import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, posing, rigging, skinning, starter


class PoseControlTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        aurora_live.register()

    @classmethod
    def tearDownClass(cls):
        aurora_live.unregister()

    def setUp(self):
        if bpy.context.object and bpy.context.object.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        bpy.context.scene.aurora_live.objects.clear()
        bpy.context.scene.unit_settings.scale_length = 1
        bpy.context.scene.aurora_live.forward = "NATIVE"
        self.rig, self.shirt = starter.create(bpy.context)
        bpy.context.scene.aurora_live.rig = self.rig

    def matrices(self):
        bpy.context.view_layer.update()
        return {b.name: b.matrix.copy() for b in self.rig.pose.bones if not b.bone.get("aurora_control")}

    def assertMatrixNear(self, a, b):
        self.assertLess(max(abs(a[i][j] - b[i][j]) for i in range(4) for j in range(4)), 0.0001)

    def test_root_setup_keeps_rest_parents_existing_animation_pose_and_export(self):
        pelvis = self.rig.pose.bones["mPelvis"]
        pelvis.rotation_mode = "XYZ"
        pelvis.rotation_euler.z = 0.3
        pelvis.location.x = 0.2
        pelvis.keyframe_insert(data_path="location", frame=1)
        rest = {b.name: (b.matrix_local.copy(), b.parent.name if b.parent else None, b.use_deform) for b in self.rig.data.bones}
        before = self.matrices()
        snapshot = exporter.snapshot([self.shirt], set(rest), 1)
        self.assertEqual(bpy.ops.aurora.create_pose_controls(kind="ROOT"), {"FINISHED"})
        posing.setup_root(bpy.context, self.rig)
        self.assertEqual(len(self.rig.data.bones), 161)
        for name, (matrix, parent, deform) in rest.items():
            bone = self.rig.data.bones[name]
            self.assertEqual((matrix, parent, deform), (bone.matrix_local, bone.parent.name if bone.parent else None, bone.use_deform))
            self.assertMatrixNear(before[name], self.matrices()[name])
        self.assertEqual(snapshot, exporter.snapshot([self.shirt], set(rest), 1))
        for name in (posing.ROOT, posing.COG):
            self.assertFalse(self.rig.data.bones[name].use_deform)
            self.assertTrue(self.rig.pose.bones[name].custom_shape)

    def test_root_moves_and_rotates_the_whole_fk_avatar_and_cog_pivots_at_pelvis(self):
        posing.setup_root(bpy.context, self.rig)
        before = self.matrices()
        root = self.rig.pose.bones[posing.ROOT]
        root.location = (0.3, -0.2, 0.1)
        root.rotation_mode = "XYZ"
        root.rotation_euler.z = 0.45
        after = self.matrices()
        for name, matrix in before.items():
            self.assertMatrixNear(root.matrix @ matrix, after[name])
        pelvis_position = after["mPelvis"].translation.copy()
        cog = self.rig.pose.bones[posing.COG]
        cog.rotation_mode = "XYZ"
        cog.rotation_euler.x = 0.2
        after = self.matrices()
        self.assertLess((after["mPelvis"].translation - pelvis_position).length, 1e-5)
        self.assertGreater((after["mHead"].translation - (root.matrix @ before["mHead"]).translation).length, 0.05)

    def test_root_and_hand_foot_ik_work_in_both_creation_orders(self):
        for root_first in (True, False):
            if not root_first:
                self.setUp()
            if root_first:
                posing.setup_root(bpy.context, self.rig)
            rigging.setup(bpy.context, self.rig)
            for key in rigging.LIMBS:
                rigging.set_ik(bpy.context, self.rig, key, True)
            if not root_first:
                posing.setup_root(bpy.context, self.rig)
            root = self.rig.pose.bones[posing.ROOT]
            root.rotation_mode = "XYZ"
            root.rotation_euler.z = 0.6
            root.location = (0.3, -0.1, 0.2)
            self.rig.pose.bones[posing.COG].location.z = -0.03
            bpy.context.view_layer.update()
            for key in rigging.LIMBS:
                control = self.rig.pose.bones[rigging.control_name(key)]
                self.assertEqual(control.parent.name, posing.ROOT)
                middle = self.rig.pose.bones[rigging.chain(self.rig, key)[1]]
                self.assertLess((middle.tail - control.matrix.translation).length, 0.004, (root_first, key))

    def test_classic_and_bento_targets_are_independent_and_disabling_keeps_the_pose(self):
        before = self.matrices()
        for key in posing.EYES:
            posing.setup_look(bpy.context, self.rig, key)
            self.assertFalse(posing.look_enabled(self.rig, key))
        self.assertEqual(before, self.matrices())
        for key in posing.EYES:
            posing.set_look(bpy.context, self.rig, key, True)
            self.rig.pose.bones[posing.target(key)].location += Vector((0, 0.4, 0.15))
            bpy.context.view_layer.update()
            for name in posing.EYES[key][1:]:
                eye = self.rig.pose.bones[name]
                direction = (self.rig.pose.bones[posing.target(key)].head - eye.head).normalized()
                self.assertGreater(eye.matrix.to_3x3().col[1].normalized().dot(direction), 0.99999)
            for other in posing.EYES:
                if other != key:
                    for name in posing.EYES[other][1:]:
                        self.assertMatrixNear(before[name], self.matrices()[name])
            posed = self.matrices()
            posing.set_look(bpy.context, self.rig, key, False)
            for name in posing.EYES[key][1:]:
                self.assertMatrixNear(posed[name], self.matrices()[name])
            # Restore manual eyes before checking independence of the other pair.
            for name in posing.EYES[key][1:]:
                self.rig.pose.bones[name].matrix_basis = Matrix.Identity(4)
            bpy.context.view_layer.update()

    def test_target_follows_root_and_can_be_centered_after_a_head_turn(self):
        for root_first in (True, False):
            if not root_first:
                self.setUp()
            if root_first:
                posing.setup_root(bpy.context, self.rig)
            posing.setup_look(bpy.context, self.rig, "CLASSIC")
            before = self.rig.pose.bones[posing.target("CLASSIC")].matrix.copy()
            if not root_first:
                posing.setup_root(bpy.context, self.rig)
            root = self.rig.pose.bones[posing.ROOT]
            root.location.x = 0.4
            root.rotation_mode = "XYZ"
            root.rotation_euler.z = 0.7
            bpy.context.view_layer.update()
            self.assertMatrixNear(root.matrix @ before, self.rig.pose.bones[posing.target("CLASSIC")].matrix)
            self.rig.pose.bones["mHead"].rotation_mode = "XYZ"
            self.rig.pose.bones["mHead"].rotation_euler.y = 0.4
            bpy.context.view_layer.update()
            posing.reset_look(bpy.context, self.rig, "CLASSIC")
            posing.set_look(bpy.context, self.rig, "CLASSIC", True)
            for name in ("mEyeLeft", "mEyeRight"):
                eye = self.rig.pose.bones[name]
                goal = self.rig.pose.bones[posing.target("CLASSIC")].head
                self.assertGreater(eye.matrix.to_3x3().col[1].normalized().dot((goal - eye.head).normalized()), 0.99999)

    def test_controls_use_canonical_aliases(self):
        for name in ("mPelvis", "mEyeLeft", "mEyeRight"):
            self.rig.data.bones[name].name = "Custom " + name
        posing.setup_root(bpy.context, self.rig)
        posing.setup_look(bpy.context, self.rig, "CLASSIC")
        posing.set_look(bpy.context, self.rig, "CLASSIC", True)
        self.assertTrue(posing.root_ready(self.rig))
        self.assertTrue(posing.look_enabled(self.rig, "CLASSIC"))

    def test_foreign_constraints_and_reserved_names_fail_before_changes(self):
        for key, joint in (("ROOT", "mPelvis"), ("CLASSIC", "mEyeLeft"), ("BENTO", "mFaceEyeAltLeft")):
            con = self.rig.pose.bones[joint].constraints.new("LIMIT_ROTATION")
            with self.assertRaisesRegex(ValueError, "already have constraints"):
                if key == "ROOT":
                    posing.setup_root(bpy.context, self.rig)
                else:
                    posing.setup_look(bpy.context, self.rig, key)
            self.rig.pose.bones[joint].constraints.remove(con)
            self.assertEqual(len(self.rig.data.bones), 159)
        self.rig.data.bones["mSpine1"].name = posing.ROOT
        with self.assertRaisesRegex(ValueError, "already used"):
            posing.setup_root(bpy.context, self.rig)
        self.assertEqual(len(self.rig.data.bones), 159)

    def test_shared_data_weight_paint_mode_and_object_selection_are_preserved(self):
        other = self.rig.copy()
        bpy.context.collection.objects.link(other)
        before_data = other.data
        selected = list(bpy.context.selected_objects)
        bpy.ops.object.mode_set(mode="WEIGHT_PAINT")
        posing.setup_root(bpy.context, self.rig)
        posing.setup_look(bpy.context, self.rig, "CLASSIC")
        self.assertEqual(self.shirt.mode, "WEIGHT_PAINT")
        self.assertEqual(selected, list(bpy.context.selected_objects))
        self.assertEqual(bpy.context.object, self.shirt)
        self.assertEqual(other.data, before_data)
        self.assertIsNot(self.rig.data, before_data)
        self.assertEqual(len(other.data.bones), 159)

    def test_hidden_controls_can_be_selected_without_showing_other_groups(self):
        posing.setup_root(bpy.context, self.rig)
        posing.setup_look(bpy.context, self.rig, "CLASSIC")
        for kind, name, group in (("ROOT", posing.ROOT, "ROOT"), ("COG", posing.COG, "ROOT"), ("CLASSIC", posing.target("CLASSIC"), "LOOK")):
            rigging.show(self.rig, "ALL", False)
            self.assertEqual(bpy.ops.aurora.select_pose_control(kind=kind), {"FINISHED"})
            self.assertEqual(bpy.context.active_pose_bone.name, name)
            self.assertEqual(self.rig.mode, "POSE")
            self.assertTrue(rigging.part_visible(self.rig, group))
            self.assertFalse(rigging.part_visible(self.rig, "BACK"))

    def test_controls_and_their_poses_never_enter_live_weights_or_rest_capture(self):
        supported = set(b.name for b in self.rig.data.bones)
        before = exporter.snapshot([self.shirt], supported, 1)
        posing.setup_root(bpy.context, self.rig)
        for key in posing.EYES:
            posing.setup_look(bpy.context, self.rig, key)
            posing.set_look(bpy.context, self.rig, key, True)
        self.rig.pose.bones[posing.ROOT].location.x = 3
        self.rig.pose.bones[posing.COG].rotation_mode = "XYZ"
        self.rig.pose.bones[posing.COG].rotation_euler.y = 0.4
        for name in (posing.ROOT, posing.COG, posing.target("CLASSIC"), posing.target("BENTO")):
            self.assertFalse(self.rig.data.bones[name].use_deform)
            self.shirt.vertex_groups.new(name=name).add([0], 0.9, "REPLACE")
        bpy.context.view_layer.update()
        self.assertEqual(before, exporter.snapshot([self.shirt], supported, 1))
        skinning.apply(bpy.context, [self.shirt], self.rig, clean=True)
        for name in (posing.ROOT, posing.COG, posing.target("CLASSIC"), posing.target("BENTO")):
            self.assertIsNone(self.shirt.vertex_groups.get(name))

    def test_optional_eye_pair_and_invalid_modes_are_handled_without_changes(self):
        for obj in bpy.context.selected_objects:
            obj.select_set(False)
        self.rig.select_set(True)
        bpy.context.view_layer.objects.active = self.rig
        bpy.ops.object.mode_set(mode="EDIT")
        with self.assertRaisesRegex(ValueError, "Leave Edit Mode"):
            posing.setup_root(bpy.context, self.rig)
        self.rig.data.edit_bones.remove(self.rig.data.edit_bones["mFaceEyeAltLeft"])
        bpy.ops.object.mode_set(mode="OBJECT")
        with self.assertRaisesRegex(ValueError, "missing"):
            posing.setup_look(bpy.context, self.rig, "BENTO")
        posing.setup_look(bpy.context, self.rig, "CLASSIC")
        self.rig.data.pose_position = "REST"
        with self.assertRaisesRegex(ValueError, "Pose Position"):
            posing.set_look(bpy.context, self.rig, "CLASSIC", True)

    def test_controls_work_with_centimetres_negative_y_and_object_transforms(self):
        bpy.context.scene.unit_settings.scale_length = 0.01
        self.rig, self.shirt = starter.create(bpy.context, forward="NEGATIVE_Y")
        self.rig.location = (12, -20, 4)
        self.rig.rotation_euler.z = 0.4
        self.rig.scale = (1.2, 0.8, 1.5)
        posing.setup_root(bpy.context, self.rig)
        posing.setup_look(bpy.context, self.rig, "CLASSIC")
        self.rig.pose.bones[posing.ROOT].location.x = 10
        posing.set_look(bpy.context, self.rig, "CLASSIC", True)
        goal = self.rig.matrix_world @ self.rig.pose.bones[posing.target("CLASSIC")].head
        for name in ("mEyeLeft", "mEyeRight"):
            eye = self.rig.pose.bones[name]
            direction = (goal - self.rig.matrix_world @ eye.head).normalized()
            forward = (self.rig.matrix_world.to_3x3() @ eye.matrix.to_3x3().col[1]).normalized()
            self.assertGreater(direction.dot(forward), 0.99999)


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(PoseControlTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_POSING_TESTS_OK", flush=True)
