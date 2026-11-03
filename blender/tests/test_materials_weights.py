"""Real Blender tests for live PBR capture and transactional automatic weights."""
import base64
import math
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, materials, starter, weights


class MaterialWeightTests(unittest.TestCase):
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
        bpy.context.scene.cursor.location = (0, 0, 0)
        bpy.context.scene.aurora_live.forward = "NATIVE"
        self.rig, self.mesh = starter.create(bpy.context)
        self.material = self.mesh.data.materials[0]
        self.shader = next(n for n in self.material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        self.names = {bone.name for bone in self.rig.data.bones}

    def capture(self):
        return exporter.snapshot([self.mesh], self.names, 1)

    def image(self, name, pixels, color=False):
        image = bpy.data.images.new(name, width=2, height=2, alpha=True)
        image.colorspace_settings.name = "sRGB" if color else "Non-Color"
        image.pixels.foreach_set(pixels)
        image.update()
        node = self.material.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        return node

    def test_principled_parameters_and_active_output_are_captured(self):
        self.material.node_tree.nodes.new("ShaderNodeBsdfPrincipled")
        self.shader.inputs["Base Color"].default_value = (0.1, 0.2, 0.3, 1)
        self.shader.inputs["Alpha"].default_value = 0.4
        self.shader.inputs["Metallic"].default_value = 0.7
        self.shader.inputs["Roughness"].default_value = 0.25
        emission = "Emission" if bpy.app.version < (4, 0, 0) else "Emission Color"
        self.shader.inputs[emission].default_value = (0.2, 0.4, 0.6, 1)
        self.shader.inputs["Emission Strength"].default_value = 2
        face = self.capture()["meshes"][0]["faces"][0]
        self.assertAlmostEqual(face["color"][0], 0.1)
        self.assertAlmostEqual(face["color"][3], 0.4)
        self.assertAlmostEqual(face["material"]["metallic"], 0.7)
        self.assertEqual(face["material"]["roughness"], 0.25)
        self.assertEqual(face["material"]["alpha_mode"], "BLEND")
        self.assertAlmostEqual(face["material"]["emissive"][2], 1.2)

    def test_rgba_images_flip_rows_and_preserve_explicit_alpha(self):
        pixels = [1,0,0,1, 0,1,0,1, 0,0,1,0.5, 1,1,0,1]
        image = self.image("PBR color", pixels, True)
        links = self.material.node_tree.links
        links.new(image.outputs["Color"], self.shader.inputs["Base Color"])
        links.new(image.outputs["Alpha"], self.shader.inputs["Alpha"])
        value = self.capture()
        index = value["meshes"][0]["faces"][0]["material"]["textures"][0]
        self.assertEqual(len(value["images"]), 1)
        raw = base64.b64decode(value["images"][index]["rgba"])
        self.assertEqual(raw[:3], bytes((0, 0, 255)))
        self.assertIn(raw[3], (127, 128))
        links.remove(self.shader.inputs["Alpha"].links[0])
        raw = base64.b64decode(self.capture()["images"][0]["rgba"])
        self.assertTrue(all(value == 255 for value in raw[3::4]))

    def test_normal_and_packed_metal_roughness_maps(self):
        image = self.image("PBR data", [0.2, 0.4, 0.6, 1] * 4)
        nodes, links = self.material.node_tree.nodes, self.material.node_tree.links
        split = nodes.new("ShaderNodeSeparateRGB" if bpy.app.version < (3, 3, 0) else "ShaderNodeSeparateColor")
        normal = nodes.new("ShaderNodeNormalMap")
        links.new(image.outputs["Color"], split.inputs[0])
        links.new(split.outputs[1], self.shader.inputs["Roughness"])
        links.new(split.outputs[2], self.shader.inputs["Metallic"])
        links.new(image.outputs["Color"], normal.inputs["Color"])
        links.new(normal.outputs["Normal"], self.shader.inputs["Normal"])
        value = self.capture()
        mat = value["meshes"][0]["faces"][0]["material"]
        self.assertEqual(mat["metallic"], 1)
        self.assertEqual(mat["roughness"], 1)
        self.assertIsNotNone(mat["textures"][1])
        orm = base64.b64decode(value["images"][mat["textures"][2]]["rgba"])
        self.assertEqual(tuple(orm[:4]), (255, 102, 153, 255))

    def test_uv_mapping_transform_matches_blender_bottom_origin(self):
        image = self.image("UV color", [0.5, 0.5, 0.5, 1] * 4, True)
        nodes, links = self.material.node_tree.nodes, self.material.node_tree.links
        uv, mapping = nodes.new("ShaderNodeTexCoord"), nodes.new("ShaderNodeMapping")
        mapping.inputs["Location"].default_value = (0.2, 0.3, 0)
        mapping.inputs["Rotation"].default_value.z = 0.25
        mapping.inputs["Scale"].default_value = (2, 3, 1)
        links.new(uv.outputs["UV"], mapping.inputs["Vector"])
        links.new(mapping.outputs["Vector"], image.inputs["Vector"])
        links.new(image.outputs["Color"], self.shader.inputs["Base Color"])
        transform = self.capture()["meshes"][0]["faces"][0]["material"]["transforms"][0]
        self.assertAlmostEqual(transform[0], 0.2 - math.sin(0.25) * 3, places=5)
        self.assertAlmostEqual(transform[1], 0.7 - math.cos(0.25) * 3, places=5)
        self.assertEqual(transform[2:], [2, 3, 0.25])

    def test_advanced_principled_inputs_are_rejected_in_both_shader_generations(self):
        names = ("Subsurface", "Clearcoat", "Sheen", "Transmission") if bpy.app.version < (4, 0, 0) else (
            "Subsurface Weight", "Coat Weight", "Sheen Weight", "Transmission Weight")
        for name in names:
            with self.subTest(socket=name):
                socket = self.shader.inputs[name]
                original = socket.default_value
                socket.default_value = 0.5
                with self.assertRaisesRegex(ValueError, "advanced Principled"):
                    self.capture()
                socket.default_value = original

    def test_legacy_separate_rgb_maps_export_the_actual_green_and_blue_channels(self):
        if not hasattr(bpy.types, "ShaderNodeSeparateRGB"):
            self.skipTest("Legacy Separate RGB was removed by Blender; Separate Color is tested separately")
        image = self.image("Legacy channels", [0.1, 0.3, 0.9, 1] * 4)
        nodes, links = self.material.node_tree.nodes, self.material.node_tree.links
        split = nodes.new("ShaderNodeSeparateRGB")
        links.new(image.outputs["Color"], split.inputs[0])
        links.new(split.outputs[1], self.shader.inputs["Roughness"])
        links.new(split.outputs[2], self.shader.inputs["Metallic"])
        value = self.capture()
        mat = value["meshes"][0]["faces"][0]["material"]
        orm = base64.b64decode(value["images"][mat["textures"][2]]["rgba"])
        self.assertIn(orm[1], (76, 77))
        self.assertIn(orm[2], (229, 230))

    def test_timer_detects_shader_changes_and_texture_paint_without_geometry_edits(self):
        image = self.image("Paint", [1, 0, 0, 1] * 4, True)
        self.material.node_tree.links.new(image.outputs["Color"], self.shader.inputs["Base Color"])
        self.material.node_tree.nodes.active = image
        submitted = []
        fake = SimpleNamespace(state=lambda: ("Connected", self.names), rest_axes=lambda: {}, take_shape=lambda ready=True, pose=False: None, take_import=lambda ready=True: None,
                               submit=submitted.append, close=lambda: None)
        aurora_live.tracking.add(bpy.context.scene, [self.mesh])
        with patch.object(aurora_live, "_client", fake):
            aurora_live._last_capture = 0
            aurora_live._tick()
            self.shader.inputs["Metallic"].default_value = 0.9
            aurora_live._last_capture = 0
            aurora_live._tick()
            self.assertEqual(len(submitted), 2)
            self.assertAlmostEqual(submitted[-1]["meshes"][0]["faces"][0]["material"]["metallic"], 0.9)
            bpy.ops.object.mode_set(mode="TEXTURE_PAINT")
            image.image.pixels.foreach_set([0, 1, 0, 1] * 4)
            image.image.update()
            aurora_live._last_capture = 0
            aurora_live._last_change = 0
            aurora_live._tick()
            self.assertEqual(len(submitted), 3)
            self.assertNotEqual(submitted[0]["images"][0]["rgba"], submitted[-1]["images"][0]["rgba"])
            bpy.ops.object.mode_set(mode="OBJECT")

    def test_procedural_input_is_reported_instead_of_silently_ignored(self):
        noise = self.material.node_tree.nodes.new("ShaderNodeTexNoise")
        self.material.node_tree.links.new(noise.outputs["Color"], self.shader.inputs["Base Color"])
        with self.assertRaisesRegex(ValueError, "Bake procedural"):
            self.capture()

    def test_viewport_material_alpha_and_linked_normal_strength(self):
        self.material.use_nodes = False
        self.material.diffuse_color = (0.1, 0.2, 0.3, 0.4)
        if self.material.use_nodes:  # Blender 5.2 always uses node materials.
            self.shader.inputs["Alpha"].default_value = 0.4
        face = self.capture()["meshes"][0]["faces"][0]
        self.assertAlmostEqual(face["color"][3], 0.4)
        self.assertEqual(face["material"]["alpha_mode"], "BLEND")
        self.material.use_nodes = True
        normal = self.material.node_tree.nodes.new("ShaderNodeNormalMap")
        value = self.material.node_tree.nodes.new("ShaderNodeValue")
        value.outputs[0].default_value = 2
        self.material.node_tree.links.new(value.outputs[0], normal.inputs["Strength"])
        self.material.node_tree.links.new(normal.outputs["Normal"], self.shader.inputs["Normal"])
        with self.assertRaisesRegex(ValueError, "strength 1"):
            self.capture()

    def test_constant_value_nodes_and_inactive_uv_maps(self):
        nodes, links = self.material.node_tree.nodes, self.material.node_tree.links
        constant = nodes.new("ShaderNodeValue")
        constant.outputs[0].default_value = 0.35
        links.new(constant.outputs[0], self.shader.inputs["Metallic"])
        before = materials.signature([self.mesh])
        self.assertAlmostEqual(self.capture()["meshes"][0]["faces"][0]["material"]["metallic"], 0.35)
        constant.outputs[0].default_value = 0.6
        self.assertNotEqual(before, materials.signature([self.mesh]))
        image = self.image("Wrong UV", [1, 1, 1, 1] * 4, True)
        uv = nodes.new("ShaderNodeUVMap")
        uv.uv_map = "Other UV"
        links.new(uv.outputs[0], image.inputs["Vector"])
        links.new(image.outputs["Color"], self.shader.inputs["Base Color"])
        with self.assertRaisesRegex(ValueError, "active UV"):
            self.capture()

    def test_float_image_buffers_and_color_to_scalar_follow_shader_color_management(self):
        image = bpy.data.images.new("Linear float buffer", width=2, height=2, float_buffer=True)
        image.colorspace_settings.name = "sRGB"
        image.pixels.foreach_set([0.5, 0.5, 0.5, 1] * 4)
        image.update()
        nodes, links = self.material.node_tree.nodes, self.material.node_tree.links
        texture = nodes.new("ShaderNodeTexImage")
        texture.image = image
        links.new(texture.outputs["Color"], self.shader.inputs["Base Color"])
        links.new(texture.outputs["Color"], self.shader.inputs["Roughness"])
        rgb = nodes.new("ShaderNodeRGB")
        rgb.outputs[0].default_value = (1, 0, 0, 1)
        links.new(rgb.outputs[0], self.shader.inputs["Metallic"])
        value = self.capture()
        mat = value["meshes"][0]["faces"][0]["material"]
        color = base64.b64decode(value["images"][mat["textures"][0]]["rgba"])
        orm = base64.b64decode(value["images"][mat["textures"][2]]["rgba"])
        self.assertEqual(color[0], 188, "linear 0.5 must be sRGB encoded")
        self.assertIn(orm[1], (127,128), "float data buffers stay linear")
        self.assertAlmostEqual(mat["metallic"], 0.2126)

    def spaced_image(self, name, spaces):
        """Base colour image in the first space this OCIO config has, or None."""
        image = bpy.data.images.new(name, width=2, height=2, alpha=True)
        for space in spaces:
            try:
                # Before the pixels: a new space can regenerate a generated image.
                image.colorspace_settings.name = space
            except TypeError:
                continue
            image.pixels.foreach_set([0.5, 0.5, 0.5, 1] * 4)
            image.update()
            node = self.material.node_tree.nodes.new("ShaderNodeTexImage")
            node.image = image
            self.material.node_tree.links.new(node.outputs["Color"], self.shader.inputs["Base Color"])
            return node
        bpy.data.images.remove(image)
        return None

    def test_linear_and_data_spaces_of_every_blender_generation_are_read(self):
        # Blender 3.x tags data "Raw" and linear "Linear" (#1455).
        tested = 0
        for space in ("Raw", "Linear", "Linear Rec.709", "Non-Color"):
            node = self.spaced_image(space, (space,))
            if node is None:
                continue
            value = self.capture()
            raw = base64.b64decode(value["images"][value["meshes"][0]["faces"][0]["material"]["textures"][0]]["rgba"])
            self.assertEqual(raw[0], 188, space + " values are linear and must be sRGB encoded")
            self.material.node_tree.nodes.remove(node)
            tested += 1
        self.assertGreaterEqual(tested, 2)

    def test_unreadable_texture_keeps_the_live_mesh_in_its_plain_color(self):
        if self.spaced_image("Log encoded", ("Filmic Log", "AgX Log", "XYZ")) is None:
            self.skipTest("no log color space in this OCIO config")
        self.material.diffuse_color = (0.2, 0.4, 0.6, 1)
        with self.assertRaisesRegex(ValueError, "color spaces"):
            self.capture()
        errors, warnings = {}, {}
        value = exporter.snapshot([self.mesh], self.names, 1, errors=errors, warnings=warnings)
        face = value["meshes"][0]["faces"][0]
        self.assertFalse(errors)
        self.assertIn("color spaces", warnings[self.mesh.as_pointer()])
        self.assertEqual(value["images"], [])
        self.assertEqual(face["material"]["textures"], [None] * 4)
        self.assertAlmostEqual(face["color"][2], 0.6, places=5)

    def test_float_alpha_buffers_are_unpremultiplied_when_the_alpha_output_is_used(self):
        image = bpy.data.images.new("Premultiplied float", width=2, height=2, float_buffer=True)
        image.colorspace_settings.name = "Linear" if bpy.app.version < (4, 0, 0) else "Linear Rec.709"
        image.pixels.foreach_set([0.25, 0.25, 0.25, 0.5] * 4)
        image.update()
        node = self.material.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        links = self.material.node_tree.links
        links.new(node.outputs["Color"], self.shader.inputs["Base Color"])
        links.new(node.outputs["Alpha"], self.shader.inputs["Alpha"])
        raw = base64.b64decode(self.capture()["images"][0]["rgba"])
        self.assertEqual(tuple(raw[:4]), (188,188,188,128))
        links.remove(self.shader.inputs["Alpha"].links[0])
        raw = base64.b64decode(self.capture()["images"][0]["rgba"])
        self.assertEqual(tuple(raw[:4]), (137,137,137,255))

    def test_auto_weights_rebind_tshirt_without_changing_the_rig_or_nonbone_groups(self):
        group = self.mesh.vertex_groups.new(name="Cloth pin")
        group.add([0], 0.37, "REPLACE")
        transform = self.mesh.matrix_world.copy()
        heads = [bone.head_local.copy() for bone in self.rig.data.bones]
        self.mesh.shape_key_add(name="Basis")
        self.mesh.shape_key_add(name="Fit")
        before = len(bpy.data.objects), len(bpy.data.armatures), len(bpy.data.meshes)
        weights.rebind(bpy.context, [self.mesh], self.rig)
        self.assertEqual(before, (len(bpy.data.objects), len(bpy.data.armatures), len(bpy.data.meshes)))
        self.assertAlmostEqual(self.mesh.vertex_groups["Cloth pin"].weight(0), 0.37)
        self.assertEqual(transform, self.mesh.matrix_world)
        self.assertEqual(heads, [bone.head_local for bone in self.rig.data.bones])
        self.assertTrue(all(bone.use_deform for bone in self.rig.data.bones))
        self.assertEqual(len(self.mesh.data.shape_keys.key_blocks), 2)
        palette = {group.index: group.name for group in self.mesh.vertex_groups if group.name in self.rig.data.bones}
        used = set()
        for vertex in self.mesh.data.vertices:
            values = [g for g in vertex.groups if g.group in palette]
            self.assertTrue(0 < len(values) <= 4)
            self.assertAlmostEqual(sum(g.weight for g in values), 1, places=5)
            used.update(palette[g.group] for g in values)
        self.assertIn("mShoulderLeft", used)
        self.assertIn("mShoulderRight", used)
        self.assertFalse(used - weights.BODY)
        self.capture()

    def test_failed_multiobject_solve_keeps_all_source_weights(self):
        before = [[(g.group, g.weight) for g in v.groups] for v in self.mesh.data.vertices]
        selection = list(bpy.context.selected_objects)
        with patch.object(weights, "_calculate", side_effect=[[[('mPelvis', 1)] for _ in self.mesh.data.vertices], ValueError("heat failed")]):
            with self.assertRaisesRegex(ValueError, "heat failed"):
                weights.rebind(bpy.context, [self.mesh, self.mesh], self.rig)
        self.assertEqual(before, [[(g.group, g.weight) for g in v.groups] for v in self.mesh.data.vertices])
        self.assertEqual(selection, list(bpy.context.selected_objects))

    def test_hidden_posed_transformed_rig_is_preserved_and_disabled_bones_are_skipped(self):
        self.rig.location = (2, -1, 0.5)
        self.rig.rotation_euler.z = 0.3
        self.rig.scale = (1.2, 1.2, 1.2)
        self.rig.pose.bones["mShoulderLeft"].rotation_mode = "XYZ"
        self.rig.pose.bones["mShoulderLeft"].rotation_euler.x = 0.4
        self.rig.data.bones["mHead"].use_deform = False
        self.rig.hide_select = True
        self.rig.hide_set(True)
        bpy.context.view_layer.update()
        before = self.mesh.matrix_world.copy()
        weights.rebind(bpy.context, [self.mesh], self.rig)
        self.assertEqual(before, self.mesh.matrix_world)
        self.assertTrue(self.rig.hide_get())
        self.assertTrue(self.rig.hide_select)
        self.assertEqual(self.rig.data.pose_position, "POSE")
        self.assertAlmostEqual(self.rig.pose.bones["mShoulderLeft"].rotation_euler.x, 0.4)
        self.assertFalse(self.rig.data.bones["mHead"].use_deform)
        self.assertNotIn("mHead", self.mesh.vertex_groups)

    def test_button_supports_weight_paint_and_does_not_modify_a_shared_mesh(self):
        other = self.mesh.copy()
        bpy.context.collection.objects.link(other)
        other.select_set(False)
        original = other.data
        before = [[(g.group, g.weight) for g in v.groups] for v in original.vertices]
        bpy.context.scene.aurora_live.rig = self.rig
        bpy.ops.object.mode_set(mode="WEIGHT_PAINT")
        self.assertTrue(bpy.ops.aurora.auto_weights.poll())
        self.assertEqual(bpy.ops.aurora.auto_weights(), {"FINISHED"})
        self.assertEqual(self.mesh.mode, "WEIGHT_PAINT")
        self.assertIsNot(self.mesh.data, other.data)
        self.assertEqual(before, [[(g.group, g.weight) for g in v.groups] for v in other.data.vertices])


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(MaterialWeightTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_MATERIAL_WEIGHT_TESTS_OK", flush=True)
