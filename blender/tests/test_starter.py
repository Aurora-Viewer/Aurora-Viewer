"""Exercise the starter with real Blender 2.93+, including deformation/export."""

import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import bpy
import bmesh
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, starter


class StarterTests(unittest.TestCase):
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
        bpy.context.scene.cursor.location = (0, 0, 0)
        bpy.context.scene.aurora_live.forward = "NATIVE"

    def test_all_bento_and_fitted_bones_keep_the_xml_hierarchy_and_heads(self):
        rig, shirt = starter.create(bpy.context, False)
        self.assertIsNone(shirt)
        self.assertEqual(len(rig.data.bones), 159)
        self.assertEqual(sum(b["sl_collision_volume"] for b in rig.data.bones), 26)
        for definition in starter.joints():
            bone = rig.data.bones[definition["name"]]
            self.assertEqual(bone.parent.name if bone.parent else None, definition["parent"])
            self.assertLess((bone.head_local - definition["head"]).length, 0.000001)
            self.assertTrue(bone.use_deform)
            self.assertEqual(bone["aurora_joint"], bone.name)
            if bpy.app.version >= (4, 0, 0):
                self.assertEqual(len(bone.collections), 1)
            else:
                self.assertEqual(sum(bone.layers), 1)
                self.assertIsNotNone(rig.pose.bones[bone.name].bone_group)
        self.assertAlmostEqual(rig.data.bones["mPelvis"].head_local.z, 1.067, places=6)
        self.assertAlmostEqual(rig.data.bones["mShoulderLeft"].head_local.z, 1.521, places=6)
        self.assertEqual(rig.data.bones["mHandIndex3Left"].parent.name, "mHandIndex2Left")
        self.assertIn("mTail6", rig.data.bones)
        self.assertIn("mHindLimb4Right", rig.data.bones)
        self.assertIn("mWing4Left", rig.data.bones)
        self.assertIn("mFaceLipUpperCenter", rig.data.bones)
        if bpy.app.version >= (4, 0, 0):
            self.assertFalse(rig.data.collections["Fitted Mesh"].is_visible)
        else:
            layers = rig["aurora_bone_layers"]
            self.assertFalse(rig.data.layers[layers["Fitted Mesh"]])
            self.assertTrue(rig.data.layers[layers["Body"]])
            self.assertTrue(rig.data.layers[layers["Bento Hands"]])
            self.assertEqual(len({rig.pose.bones[b.name].bone_group.name for b in rig.data.bones if b["sl_collision_volume"]}), 1)

    def test_tshirt_has_four_openings_quads_uvs_and_normalized_weights(self):
        rig, shirt = starter.create(bpy.context)
        self.assertIs(shirt.find_armature(), rig)
        self.assertIs(shirt.parent, rig)
        self.assertIs(bpy.context.view_layer.objects.active, shirt)
        self.assertIn(shirt, bpy.context.selected_objects)
        self.assertTrue(all(len(p.vertices) == 4 for p in shirt.data.polygons))
        self.assertTrue(all(v.groups and len(v.groups) <= 4 for v in shirt.data.vertices))
        for vertex in shirt.data.vertices:
            self.assertAlmostEqual(sum(g.weight for g in vertex.groups), 1, places=6)
        self.assertEqual(len(shirt.data.uv_layers), 1)
        self.assertTrue(all(0 <= v <= 1 for loop in shirt.data.uv_layers[0].data for v in loop.uv))
        bm = bmesh.new()
        try:
            bm.from_mesh(shirt.data)
            self.assertTrue(all(edge.is_manifold or edge.is_boundary for edge in bm.edges))
            boundary = {vertex for edge in bm.edges if edge.is_boundary for vertex in edge.verts}
            loops = []
            while boundary:
                component, pending = set(), [boundary.pop()]
                while pending:
                    vertex = pending.pop()
                    component.add(vertex)
                    for edge in vertex.link_edges:
                        other = edge.other_vert(vertex)
                        if edge.is_boundary and other in boundary:
                            boundary.remove(other)
                            pending.append(other)
                loops.append(component)
            self.assertEqual(len(loops), 4, "neck, hem and two sleeve cuffs")
            self.assertTrue(all(face.calc_area() > 0.0000001 for face in bm.faces))
            self.assertTrue(all(face.normal.x > 0 for face in bm.faces if all(v.co.x > 0 for v in face.verts)))
        finally:
            bm.free()

    def test_shoulder_pose_deforms_sleeve_but_capture_stays_in_rest_pose(self):
        rig, shirt = starter.create(bpy.context)
        names = {bone.name for bone in rig.data.bones}
        before = exporter.snapshot([shirt], names, 1)
        original = [v.co.copy() for v in shirt.data.vertices]
        rig.pose.bones["mShoulderLeft"].rotation_mode = "XYZ"
        rig.pose.bones["mShoulderLeft"].rotation_euler.x = 0.8
        bpy.context.view_layer.update()
        evaluated = shirt.evaluated_get(bpy.context.evaluated_depsgraph_get())
        deformed = evaluated.to_mesh()
        try:
            moved = [(original[v.index] - v.co).length for v in deformed.vertices if original[v.index].y > 0.35]
            self.assertGreater(max(moved), 0.08)
            right = [(original[v.index] - v.co).length for v in deformed.vertices if original[v.index].y < -0.35]
            self.assertLess(max(right), 0.00001)
        finally:
            evaluated.to_mesh_clear()
        after = exporter.snapshot([shirt], names, 2)
        self.assertEqual(before["meshes"], after["meshes"])
        self.assertGreater(len(after["meshes"][0]["joints"]), 5)
        json.dumps(after, allow_nan=False)

    def test_units_coordinates_and_cursor_placement_export_the_same_mesh(self):
        rig, shirt = starter.create(bpy.context)
        names = {bone.name for bone in rig.data.bones}
        reference = exporter.snapshot([shirt], names, 1)["meshes"][0]
        bpy.context.scene.unit_settings.scale_length = 0.01
        bpy.context.scene.cursor.location = (5, -3, 2)
        other, turned = starter.create(bpy.context, True, "NEGATIVE_Y")
        self.assertEqual(tuple(other.location), (5, -3, 2))
        self.assertAlmostEqual(other.data.bones["mPelvis"].head_local.z, 106.7, places=3)
        actual = exporter.snapshot([turned], names, 2, "NEGATIVE_Y")["meshes"][0]
        self.assertEqual(actual["joints"], reference["joints"])
        for left, right in zip(actual["faces"][0]["vertices"], reference["faces"][0]["vertices"]):
            self.assertLess((Vector(left["position"]) - Vector(right["position"])).length, 0.00001)
        for left, right in zip(actual["inverse_bind"], reference["inverse_bind"]):
            self.assertTrue(all(abs(a - b) < 0.00001 for a, b in zip(left, right)))

    def test_repeated_operator_calls_add_independent_rigs_without_changing_existing_objects(self):
        bpy.ops.mesh.primitive_cube_add(location=(3, 2, 1))
        cube = bpy.context.object
        transform = cube.matrix_world.copy()
        self.assertEqual(bpy.ops.aurora.create_sl_rig(with_tshirt=False), {"FINISHED"})
        first = bpy.context.object
        self.assertEqual(bpy.ops.aurora.create_sl_rig(with_tshirt=True), {"FINISHED"})
        shirt = bpy.context.object
        self.assertIsNot(first, shirt.parent)
        self.assertIn(cube.name, bpy.data.objects)
        self.assertEqual(cube.matrix_world, transform)
        self.assertFalse(cube.select_get())
        bpy.context.view_layer.objects.active = first
        shirt.select_set(False)
        first.select_set(True)
        bpy.ops.object.mode_set(mode="EDIT")
        self.assertFalse(bpy.ops.aurora.create_sl_rig.poll())

    def test_failed_creation_restores_selection_and_removes_partial_rig(self):
        bpy.ops.mesh.primitive_cube_add()
        cube = bpy.context.object
        before = (len(bpy.data.objects), len(bpy.data.armatures), len(bpy.data.collections))
        with patch.object(starter, "_tshirt", side_effect=RuntimeError("test failure")):
            with self.assertRaisesRegex(RuntimeError, "test failure"):
                starter.create(bpy.context)
        self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.armatures), len(bpy.data.collections)))
        self.assertIs(bpy.context.object, cube)
        self.assertEqual(bpy.context.selected_objects, [cube])

    def test_french_interface_and_translated_new_data_create_the_same_garment(self):
        view = bpy.context.preferences.view
        before = (view.language, view.use_translate_new_dataname, view.use_translate_interface)
        try:
            view.language = "fr_FR"
            view.use_translate_new_dataname = True
            view.use_translate_interface = True
            self.assertEqual(bpy.app.translations.pgettext_iface("Create SL avatar rig", "Operator"),
                             "Créer le squelette d’avatar SL")
            rig, shirt = starter.create(bpy.context)
            self.assertEqual(len(rig.data.bones), 159)
            material = shirt.data.materials[0]
            shader = next(node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
            self.assertEqual(tuple(shader.inputs["Base Color"].default_value), tuple(material.diffuse_color))
        finally:
            view.language, view.use_translate_new_dataname, view.use_translate_interface = before

    def test_creating_a_shirt_while_connected_tracks_it_without_opening_another_connection(self):
        with patch.object(aurora_live, "_client", object()), \
                patch.object(aurora_live, "Client", side_effect=AssertionError("unexpected connection")):
            self.assertEqual(bpy.ops.aurora.create_sl_rig(with_tshirt=True), {"FINISHED"})
            self.assertEqual(aurora_live.tracking.active(bpy.context.scene), [bpy.context.object])
            self.assertTrue(aurora_live._dirty)
            self.assertEqual(aurora_live._last_capture, 0)


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(StarterTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_STARTER_TESTS_OK", flush=True)
