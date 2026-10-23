"""Real Blender synchronization-list behavior, including save/load and Undo."""
from pathlib import Path
import base64
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import starter, tracking


class TrackingTests(unittest.TestCase):
    def test_source_identity_survives_rename_pause_reorder_and_repairs_duplicated_ids(self):
        scene = bpy.context.scene
        tracking.add(scene, [self.shirt, self.other])
        first = tracking.source_ids(scene)
        self.assertEqual(len(set(first.values())), 2)
        self.shirt.name = "Renamed garment"
        scene.aurora_live.objects[0].enabled = False
        scene.aurora_live.objects.move(0, 1)
        self.assertEqual(tracking.source_ids(scene), first)
        entries = scene.aurora_live.objects
        entries[0].source_id = entries[1].source_id
        repaired = tracking.source_ids(scene)
        self.assertEqual(len(set(repaired.values())), 2)
        self.assertEqual(repaired, tracking.source_ids(scene))

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
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        self.rig, self.shirt = starter.create(bpy.context)
        self.other = self.shirt.copy()
        self.other.data = self.shirt.data.copy()
        self.other.name = "Second garment"
        bpy.context.collection.objects.link(self.other)
        self.other.select_set(False)
        self.names = {bone.name for bone in self.rig.data.bones}
        self.sent = []
        self.closed = []
        self.fake = SimpleNamespace(state=lambda: ("Connected", self.names), rest_axes=lambda: {},
                                    submit=self.sent.append, close=lambda: self.closed.append(True))

    def tick(self):
        aurora_live._last_capture = 0
        aurora_live._tick()

    def select(self, obj):
        for selected in bpy.context.selected_objects:
            selected.select_set(False)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj

    def test_selection_is_additive_deduplicated_and_preserves_paused_entries(self):
        self.assertEqual(bpy.ops.aurora.live_selection(), {"FINISHED"})
        entries = bpy.context.scene.aurora_live.objects
        entries[0].enabled = False
        self.other.select_set(True)
        bpy.ops.aurora.live_selection()
        bpy.ops.aurora.live_selection()
        self.assertEqual([entry.object for entry in entries], [self.shirt, self.other])
        self.assertFalse(entries[0].enabled)
        self.assertTrue(entries[1].enabled)

    def test_pause_resume_and_remove_send_the_expected_live_meshes(self):
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        entries = bpy.context.scene.aurora_live.objects
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            self.assertEqual(len(self.sent[-1]["meshes"]), 2)
            entries[0].enabled = False
            self.tick()
            self.assertEqual([mesh["name"] for mesh in self.sent[-1]["meshes"]], [self.other.name])
            self.shirt.data.vertices[0].co.x += 0.1
            entries[0].enabled = True
            self.tick()
            self.assertEqual(len(self.sent[-1]["meshes"]), 2)
            self.assertNotEqual(self.sent[0]["meshes"][0], self.sent[-1]["meshes"][0])
            self.assertEqual(bpy.ops.aurora.live_remove(index=0), {"FINISHED"})
            self.tick()
            self.assertEqual([mesh["name"] for mesh in self.sent[-1]["meshes"]], [self.other.name])
            self.assertIn(self.shirt.name, bpy.context.scene.objects)
            bpy.ops.aurora.live_remove(index=0)
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"], [])

    def test_disabled_object_edits_do_not_trigger_export_and_the_name_stays_current(self):
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        bpy.context.scene.aurora_live.objects[1].enabled = False
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            count = len(self.sent)
            self.other.data.vertices[0].co.x += 0.2
            self.other.data.update()
            bpy.context.view_layer.update()
            self.tick()
            self.assertEqual(len(self.sent), count)
            self.shirt.name = "Renamed live shirt"
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"][0]["name"], "Renamed live shirt")
            self.assertEqual(bpy.context.scene.aurora_live.objects[0].object, self.shirt)
            self.assertEqual(bpy.context.scene.aurora_live.objects[0].name, "Renamed live shirt")

    def test_disconnect_and_reconnect_keep_the_list_including_all_paused(self):
        tracking.add(bpy.context.scene, [self.shirt])
        bpy.context.scene.aurora_live.objects[0].enabled = False
        with patch.object(aurora_live, "Client", return_value=self.fake):
            self.assertEqual(bpy.ops.aurora.live_connect(), {"FINISHED"})
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"], [])
            bpy.ops.aurora.live_disconnect()
            self.assertEqual(len(bpy.context.scene.aurora_live.objects), 1)
            self.select(self.other)
            bpy.ops.aurora.live_connect()
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"], [])
            self.assertEqual(bpy.context.scene.aurora_live.objects[0].object, self.shirt)
            bpy.ops.aurora.live_disconnect()
        self.assertEqual(len(self.closed), 2)

    def test_resuming_recaptures_pixels_edited_while_the_object_was_paused(self):
        tracking.add(bpy.context.scene, [self.shirt])
        material = self.shirt.data.materials[0]
        image = bpy.data.images.new("Paused paint", width=2, height=2)
        image.pixels.foreach_set([1, 0, 0, 1] * 4)
        image.update()
        node = material.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        shader = next(n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        material.node_tree.links.new(node.outputs["Color"], shader.inputs["Base Color"])
        entry = bpy.context.scene.aurora_live.objects[0]
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            self.assertEqual(base64.b64decode(self.sent[-1]["images"][0]["rgba"])[:3], b"\xff\x00\x00")
            entry.enabled = False
            self.tick()
            image.pixels.foreach_set([0, 1, 0, 1] * 4)
            image.update()
            entry.enabled = True
            self.tick()
            self.assertEqual(base64.b64decode(self.sent[-1]["images"][0]["rgba"])[:3], b"\x00\xff\x00")

    def test_connect_bootstraps_selection_only_when_the_list_is_empty(self):
        with patch.object(aurora_live, "Client", return_value=self.fake):
            bpy.ops.aurora.live_connect()
            self.assertEqual(tracking.active(bpy.context.scene), [self.shirt])
            bpy.ops.aurora.live_disconnect()

    def test_deleting_or_unlinking_an_object_prunes_it_and_clears_the_preview(self):
        for delete in (False, True):
            with self.subTest(delete=delete):
                obj = self.other.copy()
                bpy.context.collection.objects.link(obj)
                tracking.add(bpy.context.scene, [obj])
                with patch.object(aurora_live, "_client", self.fake):
                    self.tick()
                    if delete:
                        bpy.data.objects.remove(obj, do_unlink=True)
                    else:
                        bpy.context.collection.objects.unlink(obj)
                    self.tick()
                    self.assertEqual(self.sent[-1]["meshes"], [])
                    self.assertFalse(bpy.context.scene.aurora_live.objects)

    def test_starter_and_auto_weights_add_without_replacing_or_resuming_other_entries(self):
        tracking.add(bpy.context.scene, [self.shirt])
        bpy.context.scene.aurora_live.objects[0].enabled = False
        with patch.object(aurora_live, "_client", self.fake):
            self.select(self.other)
            bpy.ops.aurora.auto_weights()
            self.assertEqual(tracking.active(bpy.context.scene), [self.other])
            bpy.ops.aurora.create_sl_rig(with_tshirt=True)
            new_shirt = bpy.context.object
            self.assertEqual(tracking.active(bpy.context.scene), [self.other, new_shirt])
            self.assertFalse(bpy.context.scene.aurora_live.objects[0].enabled)

    def test_scene_switch_uses_its_own_list_and_settings(self):
        original = bpy.context.scene
        other_scene = bpy.data.scenes.new("Other sync scene")
        other_scene.collection.objects.link(self.other)
        tracking.add(original, [self.shirt])
        tracking.add(other_scene, [self.other])
        try:
            with patch.object(aurora_live, "_client", self.fake):
                self.tick()
                bpy.context.window.scene = other_scene
                self.tick()
                self.assertEqual([mesh["name"] for mesh in self.sent[-1]["meshes"]], [self.other.name])
        finally:
            bpy.context.window.scene = original
            bpy.data.scenes.remove(other_scene)

    def test_saved_scene_restores_entries_and_flags_but_stops_the_connection(self):
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        tracking.source_ids(bpy.context.scene)
        bpy.context.scene.aurora_live.objects[1].enabled = False
        expected = [(entry.object.name, entry.enabled, entry.source_id) for entry in bpy.context.scene.aurora_live.objects]
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "sync-list.blend")
            bpy.ops.wm.save_as_mainfile(filepath=path)
            aurora_live._client = self.fake
            bpy.ops.wm.open_mainfile(filepath=path)
        self.assertIsNone(aurora_live._client)
        self.assertEqual(self.closed, [True])
        self.assertEqual([(entry.object.name, entry.enabled, entry.source_id) for entry in bpy.context.scene.aurora_live.objects], expected)

    def test_undo_redo_restores_membership_and_resends_without_an_rna_update(self):
        bpy.context.preferences.edit.use_global_undo = True
        bpy.ops.ed.undo_push(message="Before sync entry")
        bpy.ops.aurora.live_selection()
        bpy.ops.ed.undo_push(message="After sync entry")
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            bpy.ops.ed.undo()
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"], [])
            bpy.ops.ed.redo()
            self.tick()
            self.assertEqual(len(self.sent[-1]["meshes"]), 1)


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(TrackingTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_TRACKING_TESTS_OK", flush=True)
