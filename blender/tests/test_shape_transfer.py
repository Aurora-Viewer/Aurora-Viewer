"""Native Blender coverage using a synthetic shape built by the Rust viewer.

blender --background --factory-startup --python-exit-code 1 --python
blender/tests/test_shape_transfer.py -- <shape-fixture.json>
"""
import copy
import json
from pathlib import Path
import sys
import unittest
import uuid
from unittest.mock import MagicMock, patch

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, shape_apply, shape_transfer, starter, tracking, rigging, posing

PAYLOAD = json.loads(Path(sys.argv[sys.argv.index("--") + 1]).read_text(encoding="utf-8"))


def matrix(values):
    return Matrix([values[i:i+4] for i in range(0, 16, 4)]).transposed()


class ShapeTransferTests(unittest.TestCase):
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
        bpy.context.scene.unit_settings.scale_length = 1
        bpy.context.scene.aurora_live.forward = "AUTO"

    def test_import_preserves_existing_work_and_keeps_shape_reference_out_of_live_list(self):
        old, shirt = starter.create(bpy.context)
        tracking.add(bpy.context.scene, [shirt])
        bpy.context.scene.aurora_live.objects[0].enabled = False
        before = [bone.matrix_local.copy() for bone in old.data.bones]
        rig = shape_transfer.apply(bpy.context, PAYLOAD)
        self.assertEqual(len(rig.data.bones), 159)
        self.assertEqual(json.loads(rig["aurora_shape_parameters"])["80"], 1)
        self.assertIs(bpy.context.scene.aurora_live.rig, rig)
        self.assertEqual([bone.matrix_local for bone in old.data.bones], before)
        self.assertEqual(len(bpy.context.scene.aurora_live.objects), 1)
        self.assertFalse(bpy.context.scene.aurora_live.objects[0].enabled)
        self.assertIs(shape_transfer.apply(bpy.context, PAYLOAD), rig, "a repeated delivery is idempotent")

    def test_reference_skin_round_trips_without_applying_shape_twice(self):
        rig = shape_transfer.apply(bpy.context, PAYLOAD)
        meshes = [obj for obj in rig.children if obj.type == "MESH"]
        self.assertEqual(len(meshes), 6)
        value = exporter.snapshot(meshes, set(PAYLOAD["names"]), 1)
        joints = {joint["name"]: joint for joint in PAYLOAD["joints"]}
        for mesh in value["meshes"]:
            palette = [matrix(joints[name]["inverse_bind"]).inverted() for name in mesh["joints"]]
            for name, inverse in zip(mesh["joints"], mesh["inverse_bind"]):
                error = max(abs(a - b) for a, b in zip(inverse, joints[name]["inverse_bind"]))
                self.assertLess(error, 1e-4, name)
            transforms = [world @ matrix(inverse) for world, inverse in zip(palette, mesh["inverse_bind"])]
            for face in mesh["faces"]:
                for vertex in face["vertices"]:
                    point = Vector(vertex["position"])
                    posed = sum((transforms[index] @ point * weight for index, weight in zip(vertex["joints"], vertex["weights"])), Vector())
                    self.assertLess((posed - point).length, 1e-4)
        self.assertGreater(rig.data.bones["mShoulderLeft"].head_local.x, rig.data.bones["mShoulderRight"].head_local.x)

    def test_main_thread_receive_operator_supports_explicit_undo(self):
        aurora_live._incoming_shape = PAYLOAD
        try:
            self.assertEqual(bpy.ops.aurora.receive_shape("EXEC_DEFAULT", True), {"FINISHED"})
            self.assertEqual(len([obj for obj in bpy.context.scene.objects if obj.get("aurora_shape_reference")]), 6)
        finally:
            aurora_live._incoming_shape = None

    def test_scene_units_and_cursor_are_respected(self):
        bpy.context.scene.unit_settings.scale_length = 0.01
        bpy.context.scene.cursor.location = (3, 4, 5)
        try:
            rig = shape_transfer.apply(bpy.context, PAYLOAD)
            self.assertEqual(rig.location, Vector((3, 4, 5)))
            basis = exporter.coordinate_basis(rig) @ Matrix.Scale(0.01, 4)
            for joint in PAYLOAD["joints"]:
                head = basis @ rig.data.bones[joint["name"]].head_local
                self.assertLess((head - Vector(joint["head"])).length, 1e-5)
        finally:
            bpy.context.scene.cursor.location = (0, 0, 0)
            bpy.context.scene.unit_settings.scale_length = 1

    def test_invalid_payloads_fail_before_creating_any_object(self):
        before = set(bpy.data.objects)
        for case in range(5):
            payload = copy.deepcopy(PAYLOAD)
            if case == 0:
                payload["joints"][0]["head"][0] = float("nan")
            elif case == 1:
                payload["names"][0] = "foreign"
            elif case == 2:
                payload["parts"][0]["triangles"][0][0] = 999999
            elif case == 3:
                payload["joints"][0]["inverse_bind"] = [0] * 16
            else:
                payload["parts"][0]["weights"][0] = [[0, 5]]
            with self.assertRaises(ValueError):
                shape_transfer.apply(bpy.context, payload)
            self.assertEqual(set(bpy.data.objects), before)

    def apply_payload(self):
        return dict(PAYLOAD, action="apply", id=str(uuid.uuid4()))

    def test_connect_without_selection_or_mesh_and_keep_receiving_shapes(self):
        fake = MagicMock()
        fake.state.return_value = ("Connected to Aurora", frozenset(PAYLOAD["names"]))
        fake.take_import.return_value = None
        fake.take_shape.return_value = None
        fake.rest_axes.return_value = {}
        with patch.object(aurora_live, "Client", return_value=fake):
            self.assertEqual(bpy.ops.aurora.live_connect(), {"FINISHED"})
            self.assertIs(aurora_live._client, fake)
            aurora_live._tick()
            self.assertEqual(fake.submit.call_args[0][0]["meshes"], [])
            fake.take_import.return_value = None
            fake.take_shape.return_value = PAYLOAD
            aurora_live._tick()
            self.assertTrue(bpy.context.object.get("aurora_shape_id"))
            fake.shape_result.assert_called_with(PAYLOAD["id"])

    def test_apply_without_target_reports_an_actionable_error_through_the_timer(self):
        fake = MagicMock()
        fake.state.return_value = ("Connected to Aurora", frozenset(PAYLOAD["names"]))
        fake.take_import.return_value = None
        fake.take_shape.return_value = self.apply_payload()
        fake.rest_axes.return_value = {}
        with patch.object(aurora_live, "_client", fake):
            aurora_live._tick()
        error = fake.shape_result.call_args[0][1]
        self.assertIn("Select the target SL skeleton", error)
        self.assertNotIn("Traceback", error)
        self.assertFalse(bpy.context.scene.objects)

    def test_apply_preserves_object_weights_keys_pose_controls_and_paused_list(self):
        rig, shirt = starter.create(bpy.context)
        bpy.context.scene.aurora_live.rig = rig
        posing.setup_root(bpy.context, rig)
        rigging.setup(bpy.context, rig)
        # Keep a second user of the old mesh/armature data untouched.
        other_rig = bpy.data.objects.new("Other shared rig", rig.data)
        bpy.context.scene.collection.objects.link(other_rig)
        other = bpy.data.objects.new("Other shared shirt", shirt.data)
        bpy.context.scene.collection.objects.link(other)
        shirt.shape_key_add(name="Basis")
        key = shirt.shape_key_add(name="Authored fit")
        key.data[0].co.z += 0.01
        old_key = [point.co.copy() for point in key.data]
        old_vertices = [v.co.copy() for v in shirt.data.vertices]
        old_weights = [[(group.group, group.weight) for group in vertex.groups] for vertex in shirt.data.vertices]
        constraints = [(bone.name, [(con.name, con.type) for con in bone.constraints]) for bone in rig.pose.bones]
        pose = {bone.name: bone.matrix_basis.copy() for bone in rig.pose.bones}
        tracking.add(bpy.context.scene, [shirt])
        bpy.context.scene.aurora_live.objects[0].enabled = False
        bpy.context.view_layer.objects.active = rig
        payload = self.apply_payload()
        result = shape_transfer.apply(bpy.context, payload)
        self.assertIs(result, rig)
        self.assertEqual([[ (g.group, g.weight) for g in v.groups] for v in shirt.data.vertices], old_weights)
        self.assertEqual([v.co for v in other.data.vertices], old_vertices)
        self.assertEqual([point.co for point in other.data.shape_keys.key_blocks[1].data], old_key)
        self.assertTrue(any((v.co - old).length > 1e-4 for v, old in zip(shirt.data.vertices, old_vertices)))
        self.assertEqual([(b.name, [(c.name, c.type) for c in b.constraints]) for b in rig.pose.bones], constraints)
        self.assertEqual({b.name: b.matrix_basis for b in rig.pose.bones}, pose)
        self.assertFalse(bpy.context.scene.aurora_live.objects[0].enabled)
        self.assertNotEqual(other_rig.data, rig.data)
        self.assertEqual(len(rig.data.bones), len(other_rig.data.bones))
        before = [v.co.copy() for v in shirt.data.vertices]
        shape_transfer.apply(bpy.context, dict(payload, id=str(uuid.uuid4())))
        self.assertLess(max((v.co - old).length for v, old in zip(shirt.data.vertices, before)), 1e-5)
        self.assertIs(shape_transfer.apply(bpy.context, payload), rig)

    def test_apply_respects_aliases_units_axes_and_object_placement(self):
        for unit in (1, 0.01):
            for forward in ("NATIVE", "NEGATIVE_Y"):
                bpy.context.scene.unit_settings.scale_length = unit
                rig, _ = starter.create(bpy.context, with_tshirt=False, forward=forward)
                rig.location = (4, -3, 7)
                rig.rotation_euler.z = 0.7
                rig.scale = (1.2, 0.9, 1.1)
                bpy.context.view_layer.update()
                rig.data.bones["mHead"].name = "My head"
                placement = rig.matrix_world.copy()
                shape_transfer.apply(bpy.context, self.apply_payload())
                basis = exporter.coordinate_basis(rig) @ Matrix.Scale(unit, 4)
                self.assertEqual(rig.matrix_world, placement)
                for joint in PAYLOAD["joints"]:
                    bone = rig.data.bones["My head" if joint["name"] == "mHead" else joint["name"]]
                    self.assertLess((basis @ bone.head_local - Vector(joint["head"])).length, 1e-4, joint["name"])

    def test_apply_rejects_missing_selection_wrong_rig_and_disabled_avastar_without_mutation(self):
        payload = self.apply_payload()
        with self.assertRaisesRegex(ValueError, "Select the target"):
            shape_transfer.apply(bpy.context, payload)
        rig, shirt = starter.create(bpy.context)
        bpy.context.view_layer.objects.active = shirt
        with self.assertRaisesRegex(ValueError, "Select the target"):
            shape_transfer.apply(bpy.context, payload)
        bpy.context.view_layer.objects.active = rig
        rig["avastar"] = True
        before = [bone.matrix_local.copy() for bone in rig.data.bones]
        with self.assertRaisesRegex(ValueError, "Enable Avastar"):
            shape_transfer.apply(bpy.context, payload)
        self.assertEqual([bone.matrix_local for bone in rig.data.bones], before)
        del rig["avastar"]
        rig.data.bones["mHead"].name = "Invalid"
        del rig.data.bones["Invalid"]["aurora_joint"]
        with self.assertRaisesRegex(ValueError, "compatible Second Life"):
            shape_transfer.apply(bpy.context, payload)

    def test_failed_mesh_preparation_is_atomic(self):
        rig, shirt = starter.create(bpy.context)
        second = shirt.copy()
        bpy.context.scene.collection.objects.link(second)
        bpy.context.view_layer.objects.active = rig
        original = shape_apply._geometry
        count = [0]
        def fail_second(*args):
            count[0] += 1
            if count[0] == 2:
                raise ValueError("Test preparation failure")
            return original(*args)
        before = set(bpy.data.meshes), set(bpy.data.armatures), rig.data, shirt.data
        with patch.object(shape_apply, "_geometry", side_effect=fail_second):
            with self.assertRaisesRegex(ValueError, "Test preparation"):
                shape_transfer.apply(bpy.context, self.apply_payload())
        self.assertEqual((set(bpy.data.meshes), set(bpy.data.armatures), rig.data, shirt.data), before)
        self.assertNotIn("aurora_shape_id", rig)

    def test_shaped_root_and_ik_keep_pelvis_and_foot_targets_coherent(self):
        rig, _ = starter.create(bpy.context)
        bpy.context.scene.aurora_live.rig = rig
        posing.setup_root(bpy.context, rig)
        rigging.setup(bpy.context, rig)
        bpy.context.view_layer.objects.active = rig
        shape_transfer.apply(bpy.context, self.apply_payload())
        pelvis = rig.pose.bones["mPelvis"]
        self.assertLess((pelvis.head - pelvis.bone.head_local).length, 1e-5)
        for key in ("FOOT_L", "FOOT_R"):
            rigging.set_ik(bpy.context, rig, key, True)
            target = rig.pose.bones[rigging.control_name(key)]
            target.location.z += 0.25
            bpy.context.view_layer.update()
            root, knee, ankle = (rig.pose.bones[name] for name in rigging.chain(rig, key))
            self.assertLess((knee.tail - target.head).length, 0.001)
            pole = rig.pose.bones[rigging.control_name(key, True)].head
            axis = (target.head - root.head).normalized()
            bend = knee.head - root.head
            bend -= axis * bend.dot(axis)
            toward = pole - root.head
            toward -= axis * toward.dot(axis)
            self.assertGreater(bend.normalized().dot(toward.normalized()), 0.999)

    def test_application_in_pose_mode_restores_mode_and_keeps_authored_channels(self):
        rig, _ = starter.create(bpy.context)
        bpy.context.view_layer.objects.active = rig
        rig.pose.bones["mChest"].rotation_mode = "XYZ"
        rig.pose.bones["mChest"].rotation_euler.y = 0.2
        before = rig.pose.bones["mChest"].matrix_basis.copy()
        bpy.ops.object.mode_set(mode="POSE")
        shape_transfer.apply(bpy.context, self.apply_payload())
        self.assertEqual(rig.mode, "POSE")
        self.assertEqual(rig.pose.bones["mChest"].matrix_basis, before)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ShapeTransferTests))
    if not result.wasSuccessful():
        raise RuntimeError("Shape transfer tests failed")
