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
    def test_ancestor_changes_schedule_an_automatic_static_update(self):
        self.other.modifiers.clear()
        parent = bpy.data.objects.new("Static parent", None)
        bpy.context.collection.objects.link(parent)
        self.other.parent = parent
        tracking.add(bpy.context.scene, [self.other])
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            aurora_live._dirty = False
            aurora_live._changed(bpy.context.scene, SimpleNamespace(updates=[SimpleNamespace(id=SimpleNamespace(original=parent))]))
            self.assertTrue(aurora_live._dirty)

    def test_connect_bootstraps_a_static_selection_and_sends_it(self):
        bpy.context.scene.aurora_live.objects.clear()
        self.other.modifiers.clear()
        self.other.vertex_groups.clear()
        self.select(self.other)
        with patch.object(aurora_live, "Client", return_value=self.fake):
            self.assertEqual(bpy.ops.aurora.live_connect(), {"FINISHED"})
            self.assertEqual(tracking.active(bpy.context.scene), [self.other])
            self.tick()
            self.assertEqual(self.sent[-1]["meshes"][0]["joints"], [])

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
        bpy.context.scene.aurora_live.auto_update = True
        bpy.context.scene.aurora_live.interval = 0.2
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
        self.fake = SimpleNamespace(state=lambda: ("Connected", self.names), rest_axes=lambda: {}, take_shape=lambda ready=True, pose=False: None, take_import=lambda ready=True: None,
                                    submit=self.sent.append, close=lambda: self.closed.append(True))

    def tick(self):
        aurora_live._last_capture = 0
        aurora_live._last_change = 0
        aurora_live._tick()

    def select(self, obj):
        for selected in bpy.context.selected_objects:
            selected.select_set(False)
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj

    def test_continuous_edits_wait_for_a_quiet_gap_but_manual_refresh_is_immediate(self):
        tracking.add(bpy.context.scene, [self.shirt])
        update = SimpleNamespace(updates=[SimpleNamespace(id=SimpleNamespace(original=self.shirt.data))])
        with patch.object(aurora_live, "_client", self.fake), patch.object(aurora_live, "_editing", return_value=True), patch.object(aurora_live.time, "monotonic", return_value=100) as clock:
            aurora_live._tick()
            self.shirt.data.vertices[0].co.x += 0.1
            for now in [101, 101.1, 101.2, 101.3]:
                clock.return_value = now
                aurora_live._changed(bpy.context.scene, update)
                aurora_live._tick()
                self.assertEqual(len(self.sent), 1)
            clock.return_value = 101.46
            aurora_live._tick()
            self.assertEqual(len(self.sent), 2)
            self.shirt.data.vertices[0].co.x += 0.1
            clock.return_value = 102
            aurora_live._changed(bpy.context.scene, update)
            bpy.ops.aurora.live_refresh()
            self.assertEqual(len(self.sent), 3)

    def test_animation_playback_does_not_wait_indefinitely_for_editing_to_stop(self):
        context = SimpleNamespace(mode="EDIT_MESH", window_manager=SimpleNamespace(windows=[
            SimpleNamespace(screen=SimpleNamespace(is_animation_playing=True), modal_operators=[])]))
        self.assertFalse(aurora_live._editing(context))
        context.window_manager.windows[0].screen.is_animation_playing = False
        self.assertTrue(aurora_live._editing(context))
        context.mode = "OBJECT"
        self.assertFalse(aurora_live._editing(context))
        context.window_manager.windows[0].modal_operators = [SimpleNamespace(bl_idname="TRANSFORM_OT_translate")]
        self.assertTrue(aurora_live._editing(context))

    def test_connection_feedback_redraws_only_the_sidebar_and_only_when_changed(self):
        calls = []
        sidebar = SimpleNamespace(type="UI", tag_redraw=lambda: calls.append("sidebar"))
        viewport = SimpleNamespace(type="WINDOW", tag_redraw=lambda: calls.append("viewport"))
        area = SimpleNamespace(type="VIEW_3D", regions=[sidebar, viewport], tag_redraw=lambda: calls.append("area"))
        context = SimpleNamespace(scene=bpy.context.scene,
                                  window_manager=SimpleNamespace(windows=[SimpleNamespace(screen=SimpleNamespace(areas=[area]))]))
        with patch.object(aurora_live, "_client", self.fake):
            aurora_live._redraw_status(context)
            for _ in range(20):
                aurora_live._redraw_status(context)
            self.assertEqual(calls, ["sidebar"])
            self.fake.state = lambda: ("New connection status", self.names)
            aurora_live._redraw_status(context)
            self.assertEqual(calls, ["sidebar", "sidebar"])

    def test_packed_capability_uses_detached_capture_buffers(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(self.fake, "features", return_value={"packed-geometry"}, create=True):
            self.tick()
        self.assertIn("_buffers", self.sent[-1]["meshes"][0]["faces"][0])

    def test_acknowledged_idle_frames_do_not_recapture_or_rewrite_scene_feedback(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(self.fake, "issues", side_effect=lambda: (aurora_live._revision, {}), create=True):
            with patch.object(aurora_live.exporter, "snapshot", wraps=aurora_live.exporter.snapshot) as capture:
                self.tick()
                for _ in range(10):
                    aurora_live._tick()
                    bpy.context.view_layer.update()
                self.assertEqual(capture.call_count, 1)
                self.assertFalse(aurora_live._dirty)

    def test_timer_reentry_does_not_touch_an_in_progress_capture(self):
        with patch.object(aurora_live, "_exporting", True), patch.object(aurora_live, "_update") as update:
            self.assertEqual(aurora_live._tick(), 0.1)
            update.assert_not_called()
            self.assertTrue(aurora_live._exporting)

    def test_material_changes_reuse_geometry_and_vertex_edits_invalidate_it(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(self.fake, "features", return_value={"packed-geometry"}, create=True):
            with patch.object(aurora_live.exporter, "_mesh", wraps=aurora_live.exporter._mesh) as geometry:
                self.tick()
                original = self.sent[-1]["meshes"][0]["faces"][0]["_buffers"]
                material = self.shirt.data.materials[0]
                shader = next(node for node in material.node_tree.nodes if node.type == "BSDF_PRINCIPLED")
                shader.inputs["Roughness"].default_value = 0.27
                self.tick()
                self.assertEqual(geometry.call_count, 1)
                self.assertIs(self.sent[-1]["meshes"][0]["faces"][0]["_buffers"], original)
                self.assertAlmostEqual(self.sent[-1]["meshes"][0]["faces"][0]["material"]["roughness"], 0.27)
                self.shirt.data.vertices[0].co.x += 0.1
                self.shirt.data.update()
                bpy.context.view_layer.update()
                self.tick()
                self.assertEqual(geometry.call_count, 2)

    def test_idle_texture_paint_reuses_geometry_without_resending_identical_content(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(self.fake, "features", return_value={"packed-geometry"}, create=True):
            bpy.ops.object.mode_set(mode="TEXTURE_PAINT")
            try:
                with patch.object(aurora_live.exporter, "_mesh", wraps=aurora_live.exporter._mesh) as geometry:
                    self.tick()
                    self.tick()
                    self.tick()
                    self.assertEqual(geometry.call_count, 1)
                    self.assertEqual(len(self.sent), 1)
            finally:
                bpy.ops.object.mode_set(mode="OBJECT")

    def test_automatic_updates_batch_geometry_material_and_membership_changes_until_interval(self):
        prefs = bpy.context.scene.aurora_live
        prefs.interval = 10
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        with patch.object(aurora_live, "_client", self.fake), patch.object(aurora_live.time, "monotonic", return_value=100) as clock:
            aurora_live._tick()
            self.assertEqual(len(self.sent), 1)
            self.shirt.data.vertices[0].co.x += 0.1
            self.shirt.active_material.diffuse_color = (0.2, 0.4, 0.6, 1)
            prefs.objects[1].enabled = False
            clock.return_value = 109.9
            aurora_live._tick()
            self.assertEqual(len(self.sent), 1, "list changes must respect the interval too")
            clock.return_value = 110
            aurora_live._tick()
            self.assertEqual(len(self.sent), 2)
            self.assertEqual([mesh["name"] for mesh in self.sent[-1]["meshes"]], [self.shirt.name])
            self.assertFalse(aurora_live._dirty)
            clock.return_value = 150
            aurora_live._tick()
            self.assertEqual(len(self.sent), 2, "idle scenes must not be sent periodically")

    def test_manual_mode_keeps_changes_pending_until_click_and_click_always_sends(self):
        prefs = bpy.context.scene.aurora_live
        prefs.auto_update = False
        prefs.interval = 3600
        with patch.object(aurora_live, "Client", return_value=self.fake), patch.object(aurora_live.time, "monotonic", return_value=100) as clock:
            bpy.ops.aurora.live_connect()
            aurora_live._tick()
            self.assertEqual(len(self.sent), 1, "connecting must publish the initial preview in manual mode")
            self.shirt.data.vertices[0].co.x += 0.1
            tracking.add(bpy.context.scene, [self.other])
            clock.return_value = 10000
            with patch.object(aurora_live.exporter, "snapshot", wraps=aurora_live.exporter.snapshot) as capture:
                aurora_live._tick()
                capture.assert_not_called()
            self.assertEqual(len(self.sent), 1)
            self.assertTrue(aurora_live._dirty)
            self.assertEqual(bpy.ops.aurora.live_refresh(), {"FINISHED"})
            self.assertEqual(len(self.sent), 2, "capture must finish during the operator, without waiting for the timer")
            self.assertEqual(len(self.sent[-1]["meshes"]), 2)
            self.assertEqual(bpy.ops.aurora.live_refresh(), {"FINISHED"})
            self.assertEqual(len(self.sent), 3, "a manual click must resend even unchanged content")
            self.assertGreater(self.sent[-1]["revision"], self.sent[-2]["revision"])

    def test_manual_click_bypasses_automatic_interval_and_resets_its_deadline(self):
        prefs = bpy.context.scene.aurora_live
        prefs.interval = 30
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(aurora_live.time, "monotonic", return_value=100) as clock:
            aurora_live._tick()
            clock.return_value = 101
            bpy.ops.aurora.live_refresh()
            self.assertEqual(len(self.sent), 2)
            self.shirt.data.vertices[0].co.x += 0.1
            aurora_live._request_capture()
            clock.return_value = 130.9
            aurora_live._tick()
            self.assertEqual(len(self.sent), 2)
            clock.return_value = 131
            aurora_live._tick()
            self.assertEqual(len(self.sent), 3)

    def test_manual_request_waits_for_handshake_and_is_cleared_on_disconnect(self):
        bpy.context.scene.aurora_live.auto_update = False
        ready = set()
        self.fake.state = lambda: ("Connecting", ready)
        with patch.object(aurora_live, "Client", return_value=self.fake):
            bpy.ops.aurora.live_connect()
            bpy.ops.aurora.live_refresh()
            self.assertFalse(self.sent)
            self.assertTrue(aurora_live._force_capture)
            ready.update(self.names)
            aurora_live._tick()
            self.assertEqual(len(self.sent), 1)
            self.assertFalse(aurora_live._force_capture)
            bpy.ops.aurora.live_disconnect()
            self.assertFalse(bpy.ops.aurora.live_refresh.poll())
            self.assertFalse(bpy.app.timers.is_registered(aurora_live._tick))
            self.assertIsNone(aurora_live._last_snapshot)

    def test_schedule_settings_do_not_dirty_content_and_resuming_auto_sends_pending_changes(self):
        prefs = bpy.context.scene.aurora_live
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake), patch.object(aurora_live.time, "monotonic", return_value=100) as clock:
            aurora_live._tick()
            prefs.interval = 60
            prefs.auto_update = False
            self.assertFalse(aurora_live._dirty)
            self.shirt.data.vertices[0].co.x += 0.1
            aurora_live._request_capture()
            clock.return_value = 160
            aurora_live._tick()
            self.assertEqual(len(self.sent), 1)
            prefs.auto_update = True
            aurora_live._tick()
            self.assertEqual(len(self.sent), 2)

    def test_texture_paint_sends_changed_pixels_without_repeating_idle_content(self):
        tracking.add(bpy.context.scene, [self.shirt])
        material = self.shirt.active_material
        material.use_nodes = True
        image = bpy.data.images.new("Scheduled paint", 2, 2)
        image.pixels.foreach_set([1, 0, 0, 1] * 4)
        node = material.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        shader = next(n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
        material.node_tree.links.new(node.outputs["Color"], shader.inputs["Base Color"])
        bpy.ops.object.mode_set(mode="TEXTURE_PAINT")
        try:
            with patch.object(aurora_live, "_client", self.fake):
                self.tick()
                self.tick()
                self.assertEqual(len(self.sent), 1, "idle paint mode must not transmit identical snapshots")
                image.pixels.foreach_set([0, 1, 0, 1] * 4)
                image.update()
                self.tick()
                self.assertEqual(len(self.sent), 2)
                self.assertEqual(base64.b64decode(self.sent[-1]["images"][0]["rgba"])[:3], b"\x00\xff\x00")
        finally:
            bpy.ops.object.mode_set(mode="OBJECT")

    def test_manual_click_can_retry_capture_errors(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake):
            with patch.object(aurora_live.exporter, "snapshot", side_effect=ValueError("Test capture failure")):
                bpy.ops.aurora.live_refresh()
            self.assertEqual(aurora_live._error, "Test capture failure")
            self.assertFalse(self.sent)
            bpy.ops.aurora.live_refresh()
            self.assertEqual(len(self.sent), 1)
            self.assertFalse(aurora_live._error)

    def test_bad_source_does_not_block_other_edits_and_recovers_after_repair(self):
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        material = bpy.data.materials.new("Unsupported live surface")
        material.use_nodes = True
        nodes = material.node_tree.nodes
        output = next(node for node in nodes if node.type == "OUTPUT_MATERIAL")
        diffuse = nodes.new("ShaderNodeBsdfDiffuse")
        material.node_tree.links.new(diffuse.outputs[0], output.inputs["Surface"])
        self.other.data.materials.clear()
        self.other.data.materials.append(material)
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            self.assertEqual([mesh["name"] for mesh in self.sent[-1]["meshes"]], [self.shirt.name])
            self.assertIn(self.other.name, aurora_live._error)
            self.assertIn("Principled", bpy.context.scene.aurora_live.objects[1].error)
            before = self.sent[-1]["meshes"][0]
            self.shirt.data.vertices[0].co.x += 0.2
            self.shirt.data.update()
            bpy.context.view_layer.update()
            aurora_live._dirty = True
            self.tick()
            self.assertNotEqual(before, self.sent[-1]["meshes"][0])
            self.assertIn(self.other.name, aurora_live._error)
            shader = next(node for node in nodes if node.type == "BSDF_PRINCIPLED")
            material.node_tree.links.new(shader.outputs[0], output.inputs["Surface"])
            self.tick()
            self.assertEqual(len(self.sent[-1]["meshes"]), 2)
            self.assertFalse(aurora_live._error)
            self.assertFalse(bpy.context.scene.aurora_live.objects[1].error)

    def test_material_poll_failure_does_not_stop_geometry_or_hide_the_error(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake):
            with patch.object(aurora_live.materials, "signature", side_effect=TypeError("Unsupported node value")):
                self.assertEqual(aurora_live._tick(), 0.1)
                self.assertEqual(len(self.sent), 1)
                self.assertIn("Unsupported node value", aurora_live._error)
                self.shirt.data.vertices[0].co.x += 0.2
                self.tick()
                self.assertEqual(len(self.sent), 2)
            self.tick()
            self.assertFalse(aurora_live._error)

    def test_unexpected_update_exception_keeps_timer_alive_and_recovers(self):
        tracking.add(bpy.context.scene, [self.shirt])
        with patch.object(aurora_live, "_client", self.fake):
            with patch.object(self.fake, "state", side_effect=AttributeError("Test unexpected failure")):
                with patch.object(aurora_live.traceback, "print_exc") as report:
                    self.assertEqual(aurora_live._tick(), 0.1)
                    self.assertEqual(aurora_live._tick(), 0.1)
                    report.assert_called_once()
            self.assertIn("Test unexpected failure", aurora_live._error)
            self.assertFalse(aurora_live._exporting)
            aurora_live._last_capture = 0
            self.assertEqual(aurora_live._tick(), 0.1)
            self.assertEqual(len(self.sent), 1)
            self.assertFalse(aurora_live._error)

    def test_error_is_a_row_button_with_a_stable_popup_and_copyable_details(self):
        calls = []

        class Layout:
            def __getattr__(self, name):
                def call(*args, **kwargs):
                    calls.append((name, args, kwargs))
                    return SimpleNamespace() if name == "operator" else self
                return call

        context = SimpleNamespace(scene=bpy.context.scene, preferences=bpy.context.preferences,
                                  region=SimpleNamespace(width=280))
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        entry = bpy.context.scene.aurora_live.objects[1]
        entry.error = "A detailed export error that must stay readable and copyable"
        with patch.object(aurora_live, "_error", entry.error), patch.object(aurora_live.lods, "draw"):
            aurora_live._draw_live(Layout(), context)
        self.assertFalse(any(call[0] == "label" and call[2].get("text") == entry.error for call in calls))
        calls.clear()
        tracking.AURORA_UL_sync_objects.draw_item(None, context, Layout(), None, entry, None, None, "", index=1)
        self.assertTrue(any(call[0] == "operator" and call[1][0] == "aurora.live_error" and call[2].get("icon") == "ERROR" for call in calls))
        popup = SimpleNamespace(index=1, message="", layout=Layout())
        shown = []
        manager = SimpleNamespace(invoke_props_dialog=lambda operator, width: shown.append(operator.message) or {"RUNNING_MODAL"})
        fake_context = SimpleNamespace(scene=bpy.context.scene, window_manager=manager)
        self.assertEqual(tracking.AURORA_OT_sync_error.invoke(popup, fake_context, None), {"RUNNING_MODAL"})
        expected = self.other.name + "\n\n" + entry.error
        self.assertEqual(shown, [expected])
        bpy.context.scene.aurora_live.objects.move(1, 0)
        bpy.context.scene.aurora_live.objects[0].error = "Repaired while open"
        calls.clear()
        tracking.AURORA_OT_sync_error.draw(popup, fake_context)
        self.assertTrue(any(call[0] == "operator" and call[1][0] == "aurora.live_copy_error" for call in calls))
        copier = SimpleNamespace(message=popup.message, report=lambda *_args: None)
        tracking.AURORA_OT_copy_error.execute(copier, fake_context)
        self.assertEqual(manager.clipboard, expected)

    def test_transport_errors_mark_only_the_named_source_and_ignore_old_revisions(self):
        tracking.add(bpy.context.scene, [self.shirt, self.other])
        with patch.object(aurora_live, "_client", self.fake):
            self.tick()
            source = bpy.context.scene.aurora_live.objects[1].source_id
            with patch.object(self.fake, "issues", return_value=(aurora_live._revision - 1, {source: "Old error"}), create=True):
                aurora_live._tick()
                self.assertFalse(bpy.context.scene.aurora_live.objects[1].error)
            with patch.object(self.fake, "issues", return_value=(aurora_live._revision, {source: "Transfer budget exceeded"}), create=True):
                aurora_live._tick()
                self.assertFalse(bpy.context.scene.aurora_live.objects[0].error)
                self.assertEqual(bpy.context.scene.aurora_live.objects[1].error, "Transfer budget exceeded")
            with patch.object(self.fake, "issues", return_value=(aurora_live._revision, {}), create=True):
                aurora_live._tick()
                self.assertFalse(bpy.context.scene.aurora_live.objects[1].error)

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
        bpy.context.scene.aurora_live.auto_update = False
        bpy.context.scene.aurora_live.interval = 37.5
        expected = [(entry.object.name, entry.enabled, entry.source_id) for entry in bpy.context.scene.aurora_live.objects]
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "sync-list.blend")
            bpy.ops.wm.save_as_mainfile(filepath=path)
            aurora_live._client = self.fake
            bpy.ops.wm.open_mainfile(filepath=path)
        self.assertIsNone(aurora_live._client)
        self.assertEqual(self.closed, [True])
        self.assertFalse(bpy.context.scene.aurora_live.auto_update)
        self.assertEqual(bpy.context.scene.aurora_live.interval, 37.5)
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
