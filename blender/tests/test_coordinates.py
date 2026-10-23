"""Real Blender coverage for Avastar axes and older Aurora rest meshes."""
from pathlib import Path
import sys
import unittest

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, starter


class CoordinateTests(unittest.TestCase):
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
        bpy.context.scene.aurora_live.property_unset("forward")
        bpy.context.scene.unit_settings.scale_length = 1
        bpy.context.scene.cursor.location = (0, 0, 0)
        self.names = {joint["name"] for joint in starter.joints()}

    def capture(self, objects, forward="AUTO"):
        return exporter.snapshot(objects, self.names, 1, forward)["meshes"]

    def assertMeshNear(self, a, b):
        self.assertEqual(a["joints"], b["joints"])
        self.assertEqual(len(a["faces"]), len(b["faces"]))
        for left, right in zip(a["inverse_bind"], b["inverse_bind"]):
            self.assertLess(max(abs(x - y) for x, y in zip(left, right)), 1e-5)
        for left, right in zip(a["faces"], b["faces"]):
            self.assertEqual(left["indices"], right["indices"])
            self.assertEqual(len(left["vertices"]), len(right["vertices"]))
            for va, vb in zip(left["vertices"], right["vertices"]):
                self.assertEqual(va["joints"], vb["joints"])
                self.assertEqual(va["weights"], vb["weights"])
                for field in ("position", "normal", "uv"):
                    self.assertLess((Vector(va[field]) - Vector(vb[field])).length,
                                    1e-4 if field == "normal" else 1e-5, field)

    def test_default_rig_faces_negative_y_and_has_the_same_canonical_live_mesh(self):
        self.assertEqual(bpy.context.scene.aurora_live.forward, "AUTO")
        native, reference = starter.create(bpy.context, forward="NATIVE")
        expected = self.capture([reference], "NATIVE")[0]
        rig, shirt = starter.create(bpy.context)
        self.assertGreater(rig.data.bones["mShoulderLeft"].head_local.x, rig.data.bones["mShoulderRight"].head_local.x)
        self.assertLess(rig.data.bones["mToeLeft"].head_local.y, rig.data.bones["mAnkleLeft"].head_local.y)
        self.assertMeshNear(self.capture([shirt])[0], expected)
        self.assertEqual(len(native.data.bones), len(rig.data.bones))

    def test_external_avastar_orientation_needs_no_aurora_metadata_and_can_mix_with_old_rigs(self):
        native, old = starter.create(bpy.context, forward="NATIVE")
        avastar, external = starter.create(bpy.context, forward="NEGATIVE_Y")
        for rig in (native, avastar):
            for key in list(rig.keys()):
                del rig[key]
            for bone in rig.data.bones:
                for key in list(bone.keys()):
                    del bone[key]
        before = [(obj.matrix_world.copy(), [v.co.copy() for v in obj.data.vertices]) for obj in (old, external)]
        actual = self.capture([old, external])
        reference = self.capture([old], "NATIVE")[0]
        for mesh in actual:
            self.assertMeshNear(mesh, reference)
        for obj, (matrix, vertices) in zip((old, external), before):
            self.assertEqual(obj.matrix_world, matrix)
            self.assertEqual([v.co for v in obj.data.vertices], vertices)

    def test_pose_and_object_placement_do_not_change_axis_detection(self):
        rig, shirt = starter.create(bpy.context)
        reference = self.capture([shirt])[0]
        rig.location = (3, -2, 1)
        rig.rotation_euler.z = 0.7
        rig.pose.bones["mShoulderLeft"].rotation_mode = "XYZ"
        rig.pose.bones["mShoulderLeft"].rotation_euler.z = 0.6
        bpy.context.view_layer.update()
        before = rig.matrix_world.copy()
        self.assertMeshNear(self.capture([shirt])[0], reference)
        self.assertEqual(rig.matrix_world, before)

    def test_saved_manual_enum_values_keep_their_previous_meaning(self):
        prefs = bpy.context.scene.aurora_live
        for number, identifier in ((0, "NATIVE"), (1, "NEGATIVE_Y"), (2, "AUTO")):
            prefs["forward"] = number
            self.assertEqual(prefs.forward, identifier)
        prefs.forward = "NATIVE"
        rig, shirt = starter.create(bpy.context)
        self.assertGreater(rig.data.bones["mShoulderLeft"].head_local.y, 0)
        self.assertMeshNear(self.capture([shirt])[0], self.capture([shirt], "NATIVE")[0])


if __name__ == "__main__":
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(CoordinateTests)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if not result.wasSuccessful():
        raise RuntimeError("Coordinate tests failed")
