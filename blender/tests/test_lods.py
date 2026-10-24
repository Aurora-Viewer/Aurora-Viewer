"""Native Blender tests: disposable rigged LOD preparation, no grid required."""
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest
import uuid

import bpy
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import lods, starter, tracking


def counts(bundle):
    return [sum(len(face["indices"]) // 3 for face in level["mesh"]["faces"]) for level in bundle["levels"]]


class LodTests(unittest.TestCase):
    def test_static_lods_need_no_armature_weights_or_temporary_rig(self):
        self.high.modifiers.clear()
        self.high.vertex_groups.clear()
        before = set(bpy.data.objects), set(bpy.data.meshes)
        bundle = self.prepare()
        self.assertEqual((set(bpy.data.objects), set(bpy.data.meshes)), before)
        self.assertGreater(counts(bundle)[3], counts(bundle)[0])
        for level in bundle["levels"]:
            self.assertEqual(level["mesh"]["joints"], [])
            self.assertEqual(level["mesh"]["inverse_bind"], [])
            self.assertTrue(all(v["weights"] == [0.0] * 4 for face in level["mesh"]["faces"] for v in face["vertices"]))

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
        bpy.context.scene.aurora_live.objects.clear()
        for obj in list(bpy.data.objects):
            bpy.data.objects.remove(obj, do_unlink=True)
        self.rig, self.high = starter.create(bpy.context)
        self.high.name = "Garment_high"
        tracking.add(bpy.context.scene, [self.high])
        self.source = tracking.source_ids(bpy.context.scene)[self.high.as_pointer()]
        self.entry = bpy.context.scene.aurora_live.objects[0]
        lods.initialize(self.entry)
        self.names = {joint["name"] for joint in starter.joints()}

    def prepare(self):
        return lods.prepare(bpy.context, {"id": str(uuid.uuid4()), "source": self.source}, self.names, {})

    def clone(self, name, ratio=0.5):
        obj = self.high.copy()
        obj.data = self.high.data.copy()
        obj.name = name
        bpy.context.scene.collection.objects.link(obj)
        mod = obj.modifiers.new("Authored LOD", "DECIMATE")
        mod.ratio = ratio
        return obj

    def test_auto_preserves_keys_weights_uv_materials_pose_selection_and_list(self):
        self.high.shape_key_add(name="Basis")
        key = self.high.shape_key_add(name="Fit")
        key.data[0].co.z += 0.02
        key.value = 0.5
        self.rig.pose.bones["mShoulderLeft"].rotation_mode = "XYZ"
        self.rig.pose.bones["mShoulderLeft"].rotation_euler.y = 0.3
        self.entry.enabled = False
        before = ([v.co.copy() for v in self.high.data.vertices], set(bpy.data.objects), set(bpy.data.meshes),
                  [p.co.copy() for p in key.data], bpy.context.object, list(bpy.context.selected_objects))
        bundle = self.prepare()
        self.assertEqual(counts(bundle), [126, 251, 502, 1004])
        self.assertEqual([v.co for v in self.high.data.vertices], before[0])
        self.assertEqual((set(bpy.data.objects), set(bpy.data.meshes)), before[1:3])
        self.assertEqual([p.co for p in key.data], before[3])
        self.assertEqual((bpy.context.object, list(bpy.context.selected_objects)), before[4:])
        self.assertFalse(self.entry.enabled)
        palette = bundle["levels"][3]["mesh"]["joints"]
        for level in bundle["levels"]:
            self.assertEqual(level["mesh"]["joints"], palette)
            for face in level["mesh"]["faces"]:
                for vertex in face["vertices"]:
                    self.assertAlmostEqual(sum(vertex["weights"]), 1, places=5)
                    self.assertEqual(len(vertex["uv"]), 2)
        # Fixture for the Rust decoder, importer and skin/LOD validation tests.
        if "--" in sys.argv:
            Path(sys.argv[sys.argv.index("--") + 1]).write_text(json.dumps(bundle), encoding="utf-8")

    def test_manual_assignment_overrides_name_detection_and_hidden_sources_work(self):
        manual = self.clone("Explicit garment", 0.4)
        self.clone("Garment_medium", 0.8)
        self.entry.medium.object = manual
        manual.hide_viewport = True
        result = self.prepare()
        self.assertEqual(result["levels"][2]["origin"], manual.name)
        self.assertTrue(manual.hide_viewport)
        self.assertLess(counts(result)[2], 500)

    def test_case_insensitive_suffix_detection_and_fallback_only_for_missing_objects(self):
        lower = self.clone("garment_LOW", 0.2)
        self.entry.low.mode = "EXISTING"
        result = self.prepare()
        self.assertEqual(result["levels"][1]["origin"], lower.name)
        self.assertEqual(result["levels"][0]["origin"], "Automatic")

    def test_percentage_and_triangle_budgets_are_per_object_and_saved_in_scene(self):
        self.entry.medium.budget = "TRIANGLES"
        self.entry.medium.triangles = 80
        self.entry.low.ratio = 5
        result = self.prepare()
        self.assertLessEqual(counts(result)[2], 82)
        self.assertLessEqual(counts(result)[1], 52)
        other = self.clone("Other object")
        tracking.add(bpy.context.scene, [other])
        second = bpy.context.scene.aurora_live.objects[1]
        lods.initialize(second)
        self.assertEqual(second.medium.ratio, 50)
        self.assertEqual(self.entry.medium.triangles, 80)

    def test_required_missing_object_fails_without_creating_anything(self):
        self.entry.low.mode = "EXISTING"
        before = set(bpy.data.objects), set(bpy.data.meshes)
        with self.assertRaisesRegex(ValueError, "Missing LOD"):
            self.prepare()
        self.assertEqual((set(bpy.data.objects), set(bpy.data.meshes)), before)

    def test_invalid_existing_weights_do_not_silently_generate_a_replacement(self):
        low = self.clone("Garment_low")
        low.vertex_groups.clear()
        self.entry.low.mode = "FALLBACK"
        before = set(bpy.data.objects), set(bpy.data.meshes)
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual((set(bpy.data.objects), set(bpy.data.meshes)), before)

    def test_material_order_mapping_tracks_original_materials(self):
        first = self.high.data.materials[0]
        second = bpy.data.materials.new("Second cloth")
        second.use_nodes = True
        self.high.data.materials.append(second)
        for polygon in self.high.data.polygons:
            polygon.material_index = polygon.index % 2
        low = self.clone("Garment_low", 0.5)
        indices = [polygon.material_index for polygon in low.data.polygons]
        low.data.materials.clear()
        low.data.materials.append(second)
        low.data.materials.append(first)
        for polygon, index in zip(low.data.polygons, indices):
            polygon.material_index = 1 - index
        result = self.prepare()
        self.assertEqual(result["levels"][1]["slots"], [1, 0])

    def test_source_resolution_uses_entry_id_and_retains_renamed_objects(self):
        self.high.name = "Renamed garment"
        result = self.prepare()
        self.assertEqual(result["levels"][3]["mesh"]["name"], self.high.name)
        with self.assertRaisesRegex(ValueError, "no longer"):
            lods.prepare(bpy.context, {"id": str(uuid.uuid4()), "source": str(uuid.uuid4())}, self.names, {})

    def test_saved_scene_retains_lod_policies_assignments_and_budgets(self):
        low = self.clone("Custom low")
        self.entry.low.object = low
        self.entry.low.mode = "EXISTING"
        self.entry.medium.budget, self.entry.medium.triangles = "TRIANGLES", 80
        with tempfile.TemporaryDirectory(prefix="aurora-lods-") as folder:
            path = str(Path(folder) / "lod-settings.blend")
            bpy.ops.wm.save_as_mainfile(filepath=path)
            bpy.ops.wm.open_mainfile(filepath=path)
            entry = bpy.context.scene.aurora_live.objects[0]
            self.assertEqual(entry.low.object.name, "Custom low")
            self.assertEqual(entry.low.mode, "EXISTING")
            self.assertEqual(entry.medium.triangles, 80)
            self.assertEqual(entry.medium.budget, "TRIANGLES")
            self.assertEqual(entry.source_id, self.source)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(LodTests))
    if not result.wasSuccessful():
        raise RuntimeError("LOD preparation tests failed")
