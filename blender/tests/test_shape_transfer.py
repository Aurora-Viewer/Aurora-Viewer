"""Native Blender coverage using a synthetic shape built by the Rust viewer.

blender --background --factory-startup --python-exit-code 1 --python
blender/tests/test_shape_transfer.py -- <shape-fixture.json>
"""
import copy
import json
from pathlib import Path
import sys
import unittest

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, shape_transfer, starter, tracking

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


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ShapeTransferTests))
    if not result.wasSuccessful():
        raise RuntimeError("Shape transfer tests failed")
