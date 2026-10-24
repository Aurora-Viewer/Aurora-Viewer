"""Real Blender LiveMesh grouping, independent of scene parenting."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import bpy
from mathutils import Vector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import exporter, starter, tracking


class GroupTests(unittest.TestCase):
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
        bpy.context.scene.unit_settings.scale_length = 1
        bpy.ops.object.select_all(action="SELECT")
        bpy.ops.object.delete(use_global=False)
        self.objects = []
        for index, name in enumerate(("Group A", "Group B", "Group C", "Group D")):
            bpy.ops.mesh.primitive_cube_add(size=0.5, location=(10 + index, 20 + index * 2, 30))
            obj = bpy.context.object
            obj.name = name
            self.objects.append(obj)
        self.scene = bpy.context.scene

    def entry(self, obj):
        return next(entry for entry in bpy.context.scene.aurora_live.objects if entry.object == obj)

    def add(self, *objects, root=None):
        bpy.ops.object.select_all(action="DESELECT")
        for obj in objects:
            obj.select_set(True)
        bpy.context.view_layer.objects.active = root or objects[0]
        self.assertEqual(bpy.ops.aurora.live_selection(), {"FINISHED"})

    def mark(self, *objects, focus=None):
        for index, entry in enumerate(bpy.context.scene.aurora_live.objects):
            entry.marked = entry.object in objects
            if entry.object == focus:
                bpy.context.scene.aurora_live.object_index = index

    def pick(self, obj, extend=False, range_select=False):
        index = next(index for index, entry in enumerate(self.scene.aurora_live.objects) if entry.object == obj)
        self.assertEqual(bpy.ops.aurora.live_pick(index=index, extend=extend, range_select=range_select), {"FINISHED"})

    def selected(self):
        return {entry.object for entry in self.scene.aurora_live.objects if entry.marked}

    def snapshot(self, **kwargs):
        scene = bpy.context.scene
        return exporter.snapshot(tracking.active(scene), {"mPelvis", "mChest"}, 1,
                                 source_ids=tracking.source_ids(scene), link_roots=tracking.link_roots(scene), **kwargs)

    def assert_links(self, expected):
        value = self.snapshot()
        ids = tracking.source_ids(bpy.context.scene)
        actual = {mesh["name"]: mesh.get("link", {}).get("parent") for mesh in value["meshes"]}
        self.assertEqual(actual, {obj.name: ids[root.as_pointer()] if root else None for obj, root in expected.items()})
        return value

    def fake_client(self):
        self.sent = []
        return SimpleNamespace(state=lambda: ("Connected", {"mPelvis", "mChest"}), rest_axes=lambda: {},
                               take_shape=lambda ready=True, pose=False: None, take_import=lambda ready=True: None,
                               submit=self.sent.append, close=lambda: None)

    def tick(self):
        aurora_live._last_capture = 0
        aurora_live._tick()

    def test_multiple_added_objects_use_the_active_root_without_touching_scene_transforms(self):
        a, b, c, _d = self.objects
        b.rotation_euler.z = 0.4
        b.scale = (2, 1, 3)
        bpy.context.view_layer.update()
        before = [(obj.parent, obj.matrix_world.copy()) for obj in (a, b, c)]
        self.add(a, b, c, root=b)
        value = self.assert_links({a: b, b: None, c: b})
        for obj, (parent, matrix) in zip((a, b, c), before):
            self.assertEqual(obj.parent, parent)
            self.assertEqual(obj.matrix_world, matrix)
        for obj in (a, c):
            mesh = next(mesh for mesh in value["meshes"] if mesh["name"] == obj.name)
            self.assertLess((Vector(mesh["link"]["offset"]) - (obj.location - b.location)).length, 1e-6)
        rows = tracking.rows(self.scene)
        self.assertEqual([self.scene.aurora_live.objects[index].object for index, *_rest in rows], [b, a, c])
        self.assertEqual([(number, count) for _index, _root, number, count in rows], [(1, 3), (2, 3), (3, 3)])

    def test_separate_additions_create_independent_linksets_and_keep_paused_entries(self):
        a, b, c, d = self.objects
        self.add(a, b, root=a)
        self.entry(a).enabled = False
        first_group = self.entry(a).group_id
        self.add(c, d, root=d)
        self.assertNotEqual(first_group, self.entry(c).group_id)
        self.assert_links({b: None, c: d, d: None})
        self.entry(a).enabled = True
        self.assert_links({a: None, b: a, c: d, d: None})

    def test_group_checked_entries_uses_the_highlighted_row_and_splits_previous_groups(self):
        a, b, c, d = self.objects
        self.add(a, b, root=a)
        self.add(c, d, root=c)
        self.mark(b, c, focus=b)
        self.assertEqual(bpy.ops.aurora.live_group(), {"FINISHED"})
        self.assert_links({a: None, b: None, c: b, d: None})
        self.assertTrue(all(obj.parent is None for obj in self.objects))

    def test_make_root_changes_the_whole_family_without_changing_source_ids(self):
        a, b, c, _d = self.objects
        self.add(a, b, c, root=a)
        ids = tracking.source_ids(self.scene)
        self.mark(focus=c)
        self.assertEqual(bpy.ops.aurora.live_root(), {"FINISHED"})
        self.assert_links({a: c, b: c, c: None})
        self.assertEqual(tracking.source_ids(self.scene), ids)

    def test_ungrouping_one_child_preserves_the_remaining_linkset(self):
        a, b, c, _d = self.objects
        self.add(a, b, c, root=a)
        self.mark(b)
        self.assertEqual(bpy.ops.aurora.live_ungroup(), {"FINISHED"})
        self.assert_links({a: None, b: None, c: a})

    def test_ungrouping_the_root_splits_its_complete_family(self):
        a, b, c, _d = self.objects
        self.add(a, b, c, root=a)
        self.mark(a)
        bpy.ops.aurora.live_ungroup()
        self.assert_links({a: None, b: None, c: None})

    def test_row_click_and_ctrl_click_group_only_the_selected_subset(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d, root=a)
        self.pick(b)
        self.assertEqual(self.selected(), {b})
        self.assertFalse(bpy.ops.aurora.live_group.poll())
        self.pick(d, extend=True)
        self.assertEqual(self.selected(), {b, d})
        bpy.ops.aurora.live_group()
        self.assert_links({a: None, b: d, c: a, d: None})

    def test_ctrl_click_toggles_entries_and_can_clear_the_selection(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d)
        self.pick(b)
        self.pick(d, extend=True)
        self.pick(b, extend=True)
        self.assertEqual(self.selected(), {d})
        self.assertEqual(tracking.focused(bpy.context).object, d)
        self.pick(d, extend=True)
        self.assertFalse(self.selected())
        self.assertEqual(self.scene.aurora_live.object_index, -1)
        self.assertFalse(bpy.ops.aurora.live_group.poll())
        self.assertFalse(bpy.ops.aurora.live_ungroup.poll())

    def test_shift_click_uses_display_order_with_a_root_reordered_to_the_top(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d, root=c)
        self.pick(c)
        self.pick(b, range_select=True)
        self.assertEqual(self.selected(), {c, a, b})
        self.pick(d)
        self.pick(b, extend=True)
        self.pick(c, extend=True, range_select=True)
        self.assertEqual(self.selected(), {a, b, c, d})

    def test_range_anchor_survives_backing_list_reordering(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d, root=a)
        self.pick(b)
        self.scene.aurora_live.objects.move(1, 3)
        self.pick(d, range_select=True)
        self.assertEqual(self.selected(), {b, d})

    def test_paused_entries_can_be_selected_and_grouped_without_resuming(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d)
        self.entry(b).enabled = False
        self.pick(b)
        self.pick(c, extend=True)
        bpy.ops.aurora.live_group()
        self.assertFalse(self.entry(b).enabled)
        self.assertEqual(self.entry(b).group_id, self.entry(c).group_id)
        self.assert_links({a: None, c: None, d: a})

    def test_ungroup_selected_children_preserves_the_root_and_unselected_children(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d, root=a)
        ids = tracking.source_ids(self.scene)
        self.pick(b)
        self.pick(c, extend=True)
        bpy.ops.aurora.live_ungroup()
        self.assert_links({a: None, b: None, c: None, d: a})
        self.assertEqual(tracking.source_ids(self.scene), ids)

    def test_ungroup_selected_root_does_not_affect_other_linksets(self):
        a, b, c, d = self.objects
        self.add(a, b, root=a)
        self.add(c, d, root=c)
        self.pick(a)
        bpy.ops.aurora.live_ungroup()
        self.assert_links({a: None, b: None, c: None, d: c})

    def test_ungroup_mixed_root_and_child_selection_uses_each_family_role(self):
        a, b, c, d = self.objects
        bpy.ops.mesh.primitive_cube_add(size=0.5)
        e = bpy.context.object
        self.add(a, b, root=a)
        self.add(c, d, e, root=c)
        self.pick(a)
        self.pick(d, extend=True)
        bpy.ops.aurora.live_ungroup()
        self.assert_links({a: None, b: None, c: None, d: None, e: c})

    def test_list_selection_leaves_scene_selection_and_preview_updates_alone(self):
        a, b, c, _d = self.objects
        self.add(a, b, c, root=a)
        scene_selection = set(bpy.context.selected_objects)
        scene_active = bpy.context.view_layer.objects.active
        with patch.object(aurora_live, "_client", self.fake_client()):
            aurora_live._force_capture = True
            self.tick()
            aurora_live._dirty = False
            self.pick(b)
            self.pick(c, extend=True)
            with patch.object(exporter, "snapshot", wraps=exporter.snapshot) as capture:
                self.tick()
                capture.assert_not_called()
        self.assertEqual(set(bpy.context.selected_objects), scene_selection)
        self.assertEqual(bpy.context.view_layer.objects.active, scene_active)

    def test_ungroup_overrides_blender_parenting_and_readding_does_not_regroup(self):
        a, b, c, _d = self.objects
        b.parent, c.parent = a, b
        bpy.context.view_layer.update()
        before = [obj.matrix_world.copy() for obj in (a, b, c)]
        self.add(a, b, c, root=a)
        self.mark(a, b, c)
        bpy.ops.aurora.live_ungroup()
        self.add(a, b, c, root=b)
        self.assert_links({a: None, b: None, c: None})
        self.assertEqual((b.parent, c.parent), (a, b))
        self.assertEqual([obj.matrix_world for obj in (a, b, c)], before)

    def test_inherited_children_follow_an_explicit_group_without_nested_wire_parents(self):
        a, b, c, _d = self.objects
        for obj in (a, b, c):
            self.add(obj)
        self.mark(a, c, focus=c)
        bpy.ops.aurora.live_group()
        b.parent = a
        self.assert_links({a: c, b: c, c: None})

    def test_removing_or_deleting_the_root_keeps_the_remaining_group_and_scene_objects(self):
        a, b, c, d = self.objects
        self.add(a, b, c, d, root=a)
        index = next(index for index, entry in enumerate(self.scene.aurora_live.objects) if entry.object == a)
        bpy.ops.aurora.live_remove(index=index)
        self.assertIn(a.name, self.scene.objects)
        self.assert_links({b: None, c: b, d: b})
        bpy.data.objects.remove(b, do_unlink=True)
        tracking.prune(self.scene)
        self.assert_links({c: None, d: c})
        self.assertTrue(self.entry(c).group_root)

    def test_rigged_meshes_are_kept_outside_the_static_linkset(self):
        a, b, _c, _d = self.objects
        rig, shirt = starter.create(bpy.context)
        self.add(a, b, shirt, root=shirt)
        self.assertFalse(self.entry(shirt).group_id)
        value = exporter.snapshot(tracking.active(self.scene), set(rig.data.bones.keys()), 1,
                                  source_ids=tracking.source_ids(self.scene), link_roots=tracking.link_roots(self.scene))
        by_name = {mesh["name"]: mesh for mesh in value["meshes"]}
        self.assertTrue(by_name[shirt.name]["joints"])
        self.assertNotIn("link", by_name[shirt.name])
        self.assertEqual(by_name[b.name]["link"]["parent"], by_name[a.name]["source_id"])

    def test_group_offsets_use_scene_units_and_the_selected_coordinate_basis(self):
        a, b, _c, _d = self.objects
        self.add(a, b, root=a)
        self.scene.unit_settings.scale_length = 0.1
        mesh = next(mesh for mesh in self.snapshot(forward="NEGATIVE_Y")["meshes"] if mesh["name"] == b.name)
        expected = exporter.coordinate_basis(None, "NEGATIVE_Y").to_3x3() @ (b.location - a.location) * 0.1
        self.assertLess((Vector(mesh["link"]["offset"]) - expected).length, 1e-6)

    def test_invalid_override_cycles_are_rejected_before_mesh_capture(self):
        a, b, _c, _d = self.objects
        with self.assertRaisesRegex(ValueError, "Invalid LiveMesh grouping"):
            exporter.static_roots([a, b], {a.as_pointer(): b, b.as_pointer(): a})

    def test_grouping_survives_save_load_rename_and_list_reordering(self):
        a, b, c, _d = self.objects
        self.add(a, b, c, root=b)
        a.name = "Renamed linked object"
        self.scene.aurora_live.objects.move(0, 2)
        self.entry(c).enabled = False
        tracking.source_ids(self.scene)
        expected = [(entry.object.name, entry.source_id, entry.group_id, entry.group_root, entry.enabled)
                    for entry in self.scene.aurora_live.objects]
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / "livemesh-groups.blend")
            bpy.ops.wm.save_as_mainfile(filepath=path)
            bpy.ops.wm.open_mainfile(filepath=path)
        self.assertEqual([(entry.object.name, entry.source_id, entry.group_id, entry.group_root, entry.enabled)
                          for entry in bpy.context.scene.aurora_live.objects], expected)
        self.assert_links({bpy.data.objects["Renamed linked object"]: bpy.data.objects["Group B"],
                           bpy.data.objects["Group B"]: None})

    def test_group_changes_follow_manual_update_and_group_marks_do_not_capture(self):
        a, b, _c, _d = self.objects
        self.add(a)
        self.add(b)
        self.scene.aurora_live.auto_update = False
        with patch.object(aurora_live, "_client", self.fake_client()):
            aurora_live._force_capture = True
            self.tick()
            self.mark(a, b, focus=b)
            bpy.ops.aurora.live_group()
            self.tick()
            self.assertEqual(len(self.sent), 1)
            bpy.ops.aurora.live_refresh()
            self.assertEqual(len(self.sent), 2)
            self.assertEqual(sum("link" in mesh for mesh in self.sent[-1]["meshes"]), 1)
            self.scene.aurora_live.auto_update = True
            aurora_live._dirty = False
            self.mark(a)
            with patch.object(exporter, "snapshot", wraps=exporter.snapshot) as capture:
                self.tick()
                capture.assert_not_called()

    def test_undo_redo_restores_grouping_and_publishes_the_changed_linkset(self):
        a, b, _c, _d = self.objects
        self.add(a)
        self.add(b)
        bpy.context.preferences.edit.use_global_undo = True
        bpy.ops.ed.undo_push(message="Before LiveMesh grouping")
        self.mark(a, b, focus=b)
        bpy.ops.aurora.live_group()
        bpy.ops.ed.undo_push(message="After LiveMesh grouping")
        with patch.object(aurora_live, "_client", self.fake_client()):
            aurora_live._force_capture = True
            self.tick()
            self.assertEqual(sum("link" in mesh for mesh in self.sent[-1]["meshes"]), 1)
            bpy.ops.ed.undo()
            self.tick()
            self.assertTrue(all("link" not in mesh for mesh in self.sent[-1]["meshes"]))
            bpy.ops.ed.redo()
            self.tick()
            self.assertEqual(sum("link" in mesh for mesh in self.sent[-1]["meshes"]), 1)


result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(GroupTests))
if not result.wasSuccessful():
    sys.exit(1)
print("AURORA_BLENDER_GROUP_TESTS_OK", flush=True)
