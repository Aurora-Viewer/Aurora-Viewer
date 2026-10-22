"""Run with Blender --background --factory-startup --python this_file."""

import json
import sys
import unittest
import time
from pathlib import Path
from types import SimpleNamespace

import bpy
import bmesh
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter


class ExporterTests(unittest.TestCase):
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
        bpy.context.scene.unit_settings.scale_length = 1.0
        armature = bpy.data.armatures.new("Test rig")
        self.rig = bpy.data.objects.new("Test rig", armature)
        bpy.context.collection.objects.link(self.rig)
        bpy.context.view_layer.objects.active = self.rig
        self.rig.select_set(True)
        bpy.ops.object.mode_set(mode="EDIT")
        for name, x in (("mPelvis", 0.0), ("mChest", 0.2)):
            bone = armature.edit_bones.new(name)
            bone.head = (x, 0, 1)
            bone.tail = (x, 0, 1.3)
        bpy.ops.object.mode_set(mode="OBJECT")
        data = bpy.data.meshes.new("Garment")
        data.from_pydata([(0,0,1),(0.2,0,1),(0,0.2,1)], [], [(0,1,2)])
        data.uv_layers.new(name="UVMap")
        self.mesh = bpy.data.objects.new("Garment", data)
        bpy.context.collection.objects.link(self.mesh)
        group = self.mesh.vertex_groups.new(name="mPelvis")
        group.add([0,1,2], 1.0, "REPLACE")
        modifier = self.mesh.modifiers.new("Armature", "ARMATURE")
        modifier.object = self.rig
        self.mesh.select_set(True)
        bpy.context.view_layer.objects.active = self.mesh
        bpy.context.view_layer.update()

    def capture(self):
        return exporter.snapshot([self.mesh], {"mPelvis", "mChest"}, 1)

    def test_rest_pose_uvs_and_source_are_preserved(self):
        before_objects = len(bpy.data.objects)
        before_meshes = len(bpy.data.meshes)
        before_active = bpy.context.view_layer.objects.active
        before_selection = list(bpy.context.selected_objects)
        before = [tuple(v.co) for v in self.mesh.data.vertices]
        self.rig.pose.bones["mPelvis"].location = (0,0,3)
        bpy.context.view_layer.update()
        value = self.capture()
        vertex = value["meshes"][0]["faces"][0]["vertices"][0]
        self.assertAlmostEqual(vertex["position"][2], 1.0)
        self.assertEqual(vertex["weights"], [1.0,0.0,0.0,0.0])
        self.assertEqual(len(vertex["uv"]), 2)
        self.assertEqual(len(bpy.data.objects), before_objects)
        self.assertEqual(len(bpy.data.meshes), before_meshes)
        self.assertEqual(before, [tuple(v.co) for v in self.mesh.data.vertices])
        self.assertTrue(self.mesh.modifiers[0].show_viewport)
        self.assertEqual(bpy.context.view_layer.objects.active, before_active)
        self.assertEqual(list(bpy.context.selected_objects), before_selection)
        json.dumps(value, allow_nan=False)

    def test_shape_keys_modifiers_and_edit_mode(self):
        self.mesh.shape_key_add(name="Basis")
        key = self.mesh.shape_key_add(name="Widen")
        key.data[1].co.x = 0.8
        key.value = 0.5
        key_count = len(bpy.data.shape_keys)
        solid = self.mesh.modifiers.new("Thickness", "SOLIDIFY")
        solid.thickness = 0.02
        value = self.capture()
        vertices = value["meshes"][0]["faces"][0]["vertices"]
        self.assertAlmostEqual(max(v["position"][0] for v in vertices), 0.5, places=5)
        self.assertGreater(len(vertices), 3)
        self.mesh.modifiers.remove(solid)
        bpy.ops.object.mode_set(mode="EDIT")
        self.assertEqual(self.mesh.mode, "EDIT")
        self.capture()
        self.assertEqual(self.mesh.mode, "EDIT")
        self.assertEqual(len(bpy.data.shape_keys), key_count, "capture must not leave orphan shape-key data")

    def test_weight_edits_units_and_deleted_object(self):
        chest = self.mesh.vertex_groups.new(name="mChest")
        chest.add([0,1,2], 1.0, "REPLACE")
        value = self.capture()
        vertex = value["meshes"][0]["faces"][0]["vertices"][0]
        self.assertEqual(vertex["weights"], [0.5,0.5,0.0,0.0])
        bpy.context.scene.unit_settings.scale_length = 0.01
        value = self.capture()
        self.assertAlmostEqual(value["meshes"][0]["faces"][0]["vertices"][0]["position"][2], 0.01)
        obj = self.mesh
        bpy.data.objects.remove(obj, do_unlink=True)
        self.assertEqual(exporter.snapshot([obj], {"mPelvis"}, 2)["meshes"], [])

    def test_failure_cleans_temporary_data(self):
        self.mesh.vertex_groups.clear()
        before_objects, before_meshes = len(bpy.data.objects), len(bpy.data.meshes)
        with self.assertRaises(ValueError):
            self.capture()
        self.assertEqual((len(bpy.data.objects), len(bpy.data.meshes)), (before_objects, before_meshes))

    def test_top_four_weights_are_normalized(self):
        groups = [SimpleNamespace(group=i, weight=w) for i,w in enumerate([1.0,0.1,0.8,0.4,0.6])]
        joints, weights = exporter.influences(groups, {i:str(i) for i in range(5)}, {str(i):i for i in range(5)})
        self.assertEqual(joints, [0,2,4,3])
        self.assertAlmostEqual(sum(weights), 1.0)

    def test_edit_mode_vertex_move_is_visible_without_leaving_edit_mode(self):
        bpy.ops.object.mode_set(mode="EDIT")
        edit = bmesh.from_edit_mesh(self.mesh.data)
        edit.verts.ensure_lookup_table()
        edit.verts[1].co.x = 0.7
        bmesh.update_edit_mesh(self.mesh.data)
        value = self.capture()
        vertices = value["meshes"][0]["faces"][0]["vertices"]
        self.assertAlmostEqual(max(v["position"][0] for v in vertices), 0.7, places=5)
        self.assertEqual(self.mesh.mode, "EDIT")

    def test_mirrored_normals_winding_and_coordinate_conversion(self):
        self.mesh.scale.x = -1
        bpy.context.view_layer.update()
        value = exporter.snapshot([self.mesh], {"mPelvis"}, 2, "NEGATIVE_Y")
        face = value["meshes"][0]["faces"][0]
        vertices = face["vertices"]
        a,b,c = [Vector(vertices[i]["position"]) for i in face["indices"][:3]]
        normal = Vector(vertices[0]["normal"])
        self.assertGreater((b-a).cross(c-a).normalized().dot(normal), 0.99)
        self.assertAlmostEqual(min(v["position"][0] for v in vertices), -0.2, places=5)

    def test_bone_alias_and_cleanup_after_unknown_bone(self):
        self.rig.data.bones["mPelvis"]["aurora_joint"] = "mChest"
        value = self.capture()
        self.assertEqual(value["meshes"][0]["joints"], ["mChest"])
        self.assertAlmostEqual(value["meshes"][0]["inverse_bind"][0][12], 0.0)
        self.rig.data.bones["mPelvis"]["aurora_joint"] = "missing"
        before = (len(bpy.data.objects), len(bpy.data.meshes))
        with self.assertRaises(ValueError):
            self.capture()
        self.assertEqual((len(bpy.data.objects), len(bpy.data.meshes)), before)

    def test_neutral_joint_axes_correct_collision_volume_scale(self):
        axes = [0.5,0,0,0,0,0.25,0,0,0,0,2,0,0,0,0,1]
        value = exporter.snapshot([self.mesh], {"mPelvis"}, 1, rest_axes={"mPelvis": axes})
        inverse = value["meshes"][0]["inverse_bind"][0]
        self.assertEqual(inverse[:4], [0.5,0,0,0])
        self.assertEqual(inverse[12:], [0,0,-2,1])

    def test_live_handler_captures_edits_and_deletions_without_export_loop(self):
        captures = []
        aurora_live._client = SimpleNamespace(state=lambda: ("Connected", {"mPelvis"}), rest_axes=lambda: {}, submit=captures.append, close=lambda: None)
        aurora_live._objects = [self.mesh]
        aurora_live._dirty = True
        aurora_live._last_capture = 0.0
        try:
            aurora_live._tick()
            bpy.context.view_layer.update()
            self.assertFalse(aurora_live._dirty, "temporary capture objects must not start an export loop")
            self.mesh.data.vertices[1].co.x = 0.6
            self.mesh.data.update()
            bpy.context.view_layer.update()
            self.assertTrue(aurora_live._dirty)
            aurora_live._last_capture = 0.0
            aurora_live._tick()
            self.assertEqual(len(captures), 2)
            self.assertNotEqual(captures[0]["meshes"], captures[1]["meshes"])
            bpy.data.objects.remove(self.mesh, do_unlink=True)
            aurora_live._last_capture = 0.0
            aurora_live._tick()
            self.assertEqual(captures[-1]["meshes"], [])
        finally:
            aurora_live._stop()

    def test_capture_cost_for_twenty_thousand_triangles(self):
        count = 101
        data = bpy.data.meshes.new("Dense garment")
        points = [(x*0.002,y*0.002,1.0) for y in range(count) for x in range(count)]
        faces = []
        for y in range(count-1):
            for x in range(count-1):
                a = y*count+x
                faces.extend(((a,a+1,a+count),(a+1,a+count+1,a+count)))
        data.from_pydata(points, [], faces)
        self.mesh.data = data
        group = self.mesh.vertex_groups.get("mPelvis") or self.mesh.vertex_groups.new(name="mPelvis")
        group.add(list(range(len(points))), 1.0, "REPLACE")
        elapsed = []
        for _ in range(3):
            started = time.perf_counter()
            value = self.capture()
            elapsed.append((time.perf_counter()-started)*1000)
        triangles = sum(len(face["indices"])//3 for face in value["meshes"][0]["faces"])
        self.assertEqual(triangles, 20000)
        print("AURORA_BLENDER_CAPTURE_20000_TRIANGLES_MS=" + ",".join("{:.1f}".format(value) for value in elapsed))


suite = unittest.defaultTestLoader.loadTestsFromTestCase(ExporterTests)
result = unittest.TextTestRunner(verbosity=2).run(suite)
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_EXPORTER_TESTS_OK")
