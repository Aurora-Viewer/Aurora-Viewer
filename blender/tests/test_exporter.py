"""Run with Blender --background --factory-startup --python this_file."""

import json
import sys
import unittest
import time
import uuid
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import bpy
import bmesh
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter


class ExporterTests(unittest.TestCase):
    def test_parented_static_sources_keep_separate_meshes_root_and_relative_origins(self):
        from aurora_live.client import wire_snapshot
        self.mesh.modifiers.clear()
        self.mesh.parent = None
        self.mesh.location = (20, 30, 40)
        self.mesh.rotation_euler.z = 0.5
        self.mesh.scale = (2, 1, 3)
        child = self.mesh.copy()
        child.data = self.mesh.data.copy()
        bpy.context.collection.objects.link(child)
        child.parent = self.mesh
        child.location = (1, 2, -1)
        grandchild = child.copy()
        grandchild.data = child.data.copy()
        bpy.context.collection.objects.link(grandchild)
        grandchild.parent = child
        grandchild.location = (2, 1, 0)
        bpy.context.view_layer.update()
        ids = {obj.as_pointer(): str(uuid.uuid4()) for obj in (self.mesh, child, grandchild)}
        value = exporter.snapshot([grandchild, child, self.mesh], set(), 1, source_ids=ids)
        self.assertEqual(len(value["meshes"]), 3)
        self.assertNotIn("link", value["meshes"][2])
        for obj, mesh in zip((grandchild, child), value["meshes"]):
            self.assertEqual(mesh["source_id"], ids[obj.as_pointer()])
            self.assertEqual(mesh["link"]["parent"], ids[self.mesh.as_pointer()])
            expected = obj.matrix_world.translation - self.mesh.matrix_world.translation
            self.assertLess((Vector(mesh["link"]["offset"]) - expected).length, 1e-5)
        with self.assertRaisesRegex(ValueError, "linked objects"):
            wire_snapshot(value, ["source-ids", "static-meshes"])
        self.assertEqual(wire_snapshot(value, ["source-ids", "static-meshes", "object-links"]), value)
        # A paused or unselected parent is not a dangling wire reference.
        single = exporter.snapshot([child], set(), 2, source_ids=ids)
        self.assertNotIn("link", single["meshes"][0])
        bpy.context.scene.unit_settings.scale_length = 0.1
        scaled = exporter.snapshot([child, self.mesh], set(), 3, forward="NEGATIVE_Y", source_ids=ids)
        expected = exporter.coordinate_basis(None, "NEGATIVE_Y").to_3x3() @ (child.matrix_world.translation - self.mesh.matrix_world.translation) * 0.1
        self.assertLess((Vector(scaled["meshes"][0]["link"]["offset"]) - expected).length, 1e-5)
        bpy.context.scene.unit_settings.scale_length = 1

    def test_static_mesh_evaluates_modifiers_without_weights_and_ignores_scene_translation(self):
        self.mesh.modifiers.clear()
        self.mesh.vertex_groups.clear()
        self.mesh.location = (50, 60, 70)
        self.mesh.scale = (2, 3, 4)
        modifier = self.mesh.modifiers.new("Static solidify", "SOLIDIFY")
        modifier.thickness = 0.1
        bpy.context.view_layer.update()
        before = len(bpy.data.objects), len(bpy.data.meshes)
        mesh = self.capture()["meshes"][0]
        self.assertEqual(mesh["joints"], [])
        self.assertEqual(mesh["inverse_bind"], [])
        vertices = [v for face in mesh["faces"] for v in face["vertices"]]
        self.assertTrue(all(v["weights"] == [0.0] * 4 for v in vertices))
        self.assertTrue(all(v["joints"] == [0] * 4 for v in vertices))
        self.assertLess(max(v["position"][0] for v in vertices), 1.0)
        self.assertAlmostEqual(max(v["position"][2] for v in vertices), 4.0, places=4)
        self.assertGreater(sum(len(face["indices"]) // 3 for face in mesh["faces"]), 1)
        self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.meshes)))

    def test_static_and_rigged_sources_share_a_snapshot_and_negotiate_old_viewers(self):
        from aurora_live.client import wire_snapshot
        static = self.mesh.copy()
        static.data = self.mesh.data.copy()
        static.name = "Static object"
        static.modifiers.clear()
        bpy.context.collection.objects.link(static)
        value = exporter.snapshot([self.mesh, static], {"mPelvis", "mChest"}, 1)
        self.assertTrue(value["meshes"][0]["joints"])
        self.assertEqual(value["meshes"][1]["joints"], [])
        with self.assertRaisesRegex(ValueError, "Update Aurora Viewer"):
            wire_snapshot(value, ["source-ids"])
        self.assertEqual(wire_snapshot(value, ["static-meshes", "source-ids"]), value)

    def test_bone_parented_prop_without_deformation_remains_static(self):
        self.mesh.modifiers.clear()
        self.mesh.parent = self.rig
        self.mesh.parent_type = "BONE"
        self.mesh.parent_bone = "mChest"
        bpy.context.view_layer.update()
        self.assertEqual(self.capture()["meshes"][0]["joints"], [])

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

    def assert_bulk_matches_legacy(self, supported=None):
        from aurora_live.client import wire_snapshot
        supported = supported or {"mPelvis", "mChest"}
        features = ["pbr-materials", "source-ids", "static-meshes", "object-links", "packed-geometry"]
        legacy = exporter.snapshot([self.mesh], supported, 1)
        bulk = exporter.snapshot([self.mesh], supported, 1, packed=True)
        self.assertEqual(wire_snapshot(legacy, features), wire_snapshot(bulk, features))
        self.assertTrue(all(isinstance(face["_buffers"][0], bytes) for face in bulk["meshes"][0]["faces"]))
        return bulk

    def test_bulk_capture_matches_rigged_static_modifiers_and_missing_weights(self):
        self.assert_bulk_matches_legacy()
        self.mesh.vertex_groups["mPelvis"].remove([2])
        self.mesh.vertex_groups.new(name="mChest").add([1], 0.5, "REPLACE")
        self.assert_bulk_matches_legacy()
        self.mesh.vertex_groups.clear()
        self.assert_bulk_matches_legacy()
        self.mesh.vertex_groups.new(name="mPelvis").add([0, 1, 2], 1e-8, "REPLACE")
        self.assert_bulk_matches_legacy()
        self.mesh.modifiers.clear()
        self.mesh.scale = (-2, 3, 4)
        modifier = self.mesh.modifiers.new("Bulk solidify", "SOLIDIFY")
        modifier.thickness = 0.1
        bpy.context.view_layer.update()
        self.assert_bulk_matches_legacy()

    def test_bulk_capture_keeps_the_strongest_four_weights_and_tie_order(self):
        names = ["mPelvis", "mChest", "mHipLeft", "mHipRight", "mKneeLeft", "mKneeRight"]
        bpy.context.view_layer.objects.active = self.rig
        bpy.ops.object.mode_set(mode="EDIT")
        for name in names[2:]:
            bone = self.rig.data.edit_bones.new(name)
            bone.head, bone.tail = (0, 0, 1), (0, 0, 1.3)
        bpy.ops.object.mode_set(mode="OBJECT")
        self.mesh.vertex_groups.clear()
        for name, weight in zip(names, [0.3, 0.3, 0.2, 0.1, 0.05, 0.04]):
            self.mesh.vertex_groups.new(name=name).add([0, 1, 2], weight, "REPLACE")
        self.assert_bulk_matches_legacy(set(names))

    def test_bulk_capture_preserves_data_shape_keys_and_edit_mode(self):
        self.mesh.shape_key_add(name="Basis")
        shape = self.mesh.shape_key_add(name="Authored shape")
        shape.data[0].co.z += 0.2
        shape.value = 0.6
        before_data, before_users = self.mesh.data, self.mesh.data.users
        self.assert_bulk_matches_legacy()
        bpy.context.view_layer.objects.active = self.mesh
        bpy.ops.object.mode_set(mode="EDIT")
        try:
            self.assert_bulk_matches_legacy()
            self.assertEqual(bpy.context.mode, "EDIT_MESH")
            self.assertEqual(self.mesh.data, before_data)
            self.assertEqual(self.mesh.data.users, before_users)
            self.assertAlmostEqual(shape.value, 0.6)
        finally:
            bpy.ops.object.mode_set(mode="OBJECT")

    def test_direct_bulk_capture_reads_edit_mesh_without_changing_the_source(self):
        bpy.context.view_layer.objects.active = self.mesh
        bpy.ops.object.mode_set(mode="EDIT")
        try:
            mesh = bmesh.from_edit_mesh(self.mesh.data)
            mesh.verts.ensure_lookup_table()
            mesh.verts[0].co.z += 0.15
            bmesh.update_edit_mesh(self.mesh.data)
            before = (len(bpy.data.objects), len(bpy.data.meshes), self.mesh.data.users)
            self.assert_bulk_matches_legacy()
            self.assertEqual(bpy.context.mode, "EDIT_MESH")
            self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.meshes), self.mesh.data.users))
        finally:
            bpy.ops.object.mode_set(mode="OBJECT")

    def test_bulk_capture_enforces_the_same_corner_budget_and_cleans_temporary_data(self):
        before = len(bpy.data.objects), len(bpy.data.meshes)
        with patch.object(exporter, "MAX_CORNERS", 2):
            errors = {}
            value = exporter.snapshot([self.mesh], {"mPelvis", "mChest"}, 1, errors=errors, packed=True)
        self.assertFalse(value["meshes"])
        self.assertIn(self.mesh.as_pointer(), errors)
        self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.meshes)))

    def test_material_refresh_matches_full_capture_and_reuses_geometry(self):
        from aurora_live.client import wire_snapshot
        supported = {"mPelvis", "mChest"}
        ids = {self.mesh.as_pointer(): str(uuid.uuid4())}
        previous = exporter.snapshot([self.mesh], supported, 1, source_ids=ids, packed=True)
        material = bpy.data.materials.new("Refreshed material")
        material.diffuse_color = (0.1, 0.3, 0.8, 1)
        self.mesh.data.materials.append(material)
        errors = {}
        refreshed = exporter.material_snapshot([self.mesh], previous, ids, 2, errors)
        full = exporter.snapshot([self.mesh], supported, 2, source_ids=ids, packed=True)
        features = ["source-ids", "packed-geometry", "pbr-materials"]
        self.assertEqual(wire_snapshot(refreshed, features), wire_snapshot(full, features))
        self.assertIs(refreshed["meshes"][0]["faces"][0]["_buffers"], previous["meshes"][0]["faces"][0]["_buffers"])
        self.assertFalse(errors)
        self.assertIsNone(exporter.material_snapshot([], previous, ids, 3, {}))

    def test_material_refresh_keeps_source_errors_isolated(self):
        ids = {self.mesh.as_pointer(): str(uuid.uuid4())}
        previous = exporter.snapshot([self.mesh], {"mPelvis", "mChest"}, 1, source_ids=ids, packed=True)
        material = bpy.data.materials.new("Broken refresh")
        material.use_nodes = True
        output = next(node for node in material.node_tree.nodes if node.type == "OUTPUT_MATERIAL")
        diffuse = material.node_tree.nodes.new("ShaderNodeBsdfDiffuse")
        material.node_tree.links.new(diffuse.outputs[0], output.inputs["Surface"])
        self.mesh.data.materials.append(material)
        errors = {}
        value = exporter.material_snapshot([self.mesh], previous, ids, 2, errors)
        self.assertFalse(value["meshes"])
        self.assertIn("Principled", errors[self.mesh.as_pointer()])
        self.assertEqual(len(previous["meshes"]), 1)

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
        before_objects, before_meshes = len(bpy.data.objects), len(bpy.data.meshes)
        with self.assertRaises(ValueError):
            exporter.snapshot([self.mesh], {"mChest"}, 1)
        self.assertEqual((len(bpy.data.objects), len(bpy.data.meshes)), (before_objects, before_meshes))

    def test_live_failure_rolls_back_images_and_keeps_other_sources_valid(self):
        bad = self.mesh.copy()
        bad.data = self.mesh.data.copy()
        bad.name = "Broken material source"
        bpy.context.collection.objects.link(bad)
        for obj, broken in ((bad, True), (self.mesh, False)):
            material = bpy.data.materials.new("Live textured surface")
            material.use_nodes = True
            shader = next(node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
            image = bpy.data.images.new("Live rollback texture", 2, 2)
            image.pixels.foreach_set(([1, 0, 0, 1] if broken else [0, 1, 0, 1]) * 4)
            texture = material.node_tree.nodes.new("ShaderNodeTexImage")
            texture.image = image
            material.node_tree.links.new(texture.outputs["Color"], shader.inputs["Base Color"])
            if broken:
                material.node_tree.links.new(texture.outputs["Color"], shader.inputs["Normal"])
            obj.data.materials.clear()
            obj.data.materials.append(material)
        bpy.context.view_layer.update()
        before = len(bpy.data.objects), len(bpy.data.meshes)
        errors = {}
        value = exporter.snapshot([bad, self.mesh], {"mPelvis", "mChest"}, 1, errors=errors)
        self.assertEqual([mesh["name"] for mesh in value["meshes"]], [self.mesh.name])
        self.assertIn("Normal Map", errors[bad.as_pointer()])
        self.assertEqual(len(value["images"]), 1)
        self.assertEqual(value["meshes"][0]["faces"][0]["material"]["textures"][0], 0)
        self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.meshes)))
        with self.assertRaisesRegex(ValueError, "Normal Map"):
            exporter.snapshot([bad, self.mesh], {"mPelvis", "mChest"}, 2)

    def test_live_unexpected_source_error_does_not_leave_a_dangling_root(self):
        self.mesh.modifiers.clear()
        child = self.mesh.copy()
        child.data = self.mesh.data.copy()
        child.name = "Valid child"
        child.parent = self.mesh
        bpy.context.collection.objects.link(child)
        bpy.context.view_layer.update()
        ids = {obj.as_pointer(): str(uuid.uuid4()) for obj in (self.mesh, child)}
        capture = exporter._mesh

        def fail_root(source, *args):
            if source == self.mesh:
                raise TypeError("Test unexpected source error")
            return capture(source, *args)

        errors = {}
        with patch.object(exporter, "_mesh", side_effect=fail_root):
            value = exporter.snapshot([self.mesh, child], set(), 1, source_ids=ids, errors=errors)
        self.assertEqual(errors, {self.mesh.as_pointer(): "Test unexpected source error"})
        self.assertEqual([mesh["name"] for mesh in value["meshes"]], [child.name])
        self.assertNotIn("link", value["meshes"][0])

    def test_unweighted_vertices_inherit_the_nearest_weights_without_changing_source_groups(self):
        self.mesh.vertex_groups["mPelvis"].remove([1, 2])
        chest = self.mesh.vertex_groups.new(name="mChest")
        chest.add([1], 1.0, "REPLACE")
        self.mesh.vertex_groups.new(name="Non-bone mask").add([2], 1.0, "REPLACE")
        self.mesh.data.vertices[2].co = (0.19, 0.01, 1)
        self.mesh.data.update()
        before = [[(g.group, g.weight) for g in v.groups] for v in self.mesh.data.vertices]
        mesh = self.capture()["meshes"][0]
        vertices = [v for face in mesh["faces"] for v in face["vertices"]]
        missing = next(v for v in vertices if abs(v["position"][0] - 0.19) < 1e-5)
        self.assertEqual(mesh["joints"][missing["joints"][0]], "mChest")
        self.assertEqual(missing["weights"], [1.0, 0.0, 0.0, 0.0])
        self.assertEqual(before, [[(g.group, g.weight) for g in v.groups] for v in self.mesh.data.vertices])
        self.assertEqual(self.capture()["meshes"], [mesh], "idle completion must be deterministic")
        json.dumps(mesh, allow_nan=False)

    def test_wholly_unweighted_rigged_mesh_uses_the_pelvis_without_creating_groups(self):
        self.mesh.vertex_groups.clear()
        mesh = self.capture()["meshes"][0]
        self.assertEqual(mesh["joints"], ["mPelvis"])
        self.assertEqual(len(mesh["inverse_bind"]), 1)
        self.assertTrue(all(v["weights"] == [1.0, 0.0, 0.0, 0.0] for f in mesh["faces"] for v in f["vertices"]))
        self.assertEqual(len(self.mesh.vertex_groups), 0)

    def test_unweighted_completion_copies_normalized_top_four_and_rejects_nonfinite_data(self):
        joints = [0, 2, 4, 3]
        values = [0.4, 0.3, 0.2, 0.1]
        weights = {0: (joints, values), 1: ([0] * 4, [0.0] * 4)}
        exporter._complete_weights(weights, [(0, 0, 0), (1, 1, 1)])
        self.assertEqual(weights[1], (joints, values))
        for invalid in (float("inf"), float("nan")):
            with self.assertRaises(ValueError):
                exporter.influences([SimpleNamespace(group=0, weight=invalid)], {0: "mPelvis"}, {"mPelvis": 0})

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
        aurora_live._client = SimpleNamespace(state=lambda: ("Connected", {"mPelvis"}), rest_axes=lambda: {}, take_shape=lambda ready=True, pose=False: None, take_import=lambda ready=True: None,
                                             submit=captures.append, close=lambda: None)
        aurora_live.tracking.add(bpy.context.scene, [self.mesh])
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
            aurora_live._last_change = 0.0
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
