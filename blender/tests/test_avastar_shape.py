"""Optional offline integration: Blender 3.1 --factory-startup --python this.py
-- <synthetic-shape.json> <directory-containing-the-enabled-version-of-avastar>.
Use disposable, pre-created BLENDER_USER_* directories; never an account profile.
No vendor implementation or licensed assets are distributed by Aurora.
"""
import json
from pathlib import Path
import socket
import sys

import bpy

fixture, addons = sys.argv[sys.argv.index("--") + 1:]
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.append(addons)


def offline(*_args, **_kwargs):
    raise RuntimeError("External networking disabled in Avastar integration test")


socket.socket.connect = offline
import avastar
import aurora_live
from aurora_live import exporter, shape_transfer

bpy.context.preferences.use_preferences_save = False
bpy.context.preferences.addons.new().module = "avastar"
avastar.register()
aurora_live.register()
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
assert bpy.ops.avastar.add_avatar(rigType="EXTENDED", jointType="PIVOT", skeletonType="AVASTAR", no_mesh=False) == {"FINISHED"}
rig = next(obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE" and "avastar" in obj)
for obj in bpy.context.selected_objects:
    obj.select_set(False)
rig.select_set(True)
bpy.context.view_layer.objects.active = rig
payload = json.loads(Path(fixture).read_text(encoding="utf-8"))
payload["action"] = "apply"
before = {bone.name: [(con.name, con.type) for con in bone.constraints] for bone in rig.pose.bones}
head = rig.data.bones["mHead"].head_local.copy()
bpy.context.scene.SceneProp.panel_appearance_enabled = False
try:
    shape_transfer.apply(bpy.context, payload)
except ValueError as error:
    assert "appearance sliders" in str(error)
else:
    raise AssertionError("Disabled Avastar deformation must not report success")
assert "aurora_shape_id" not in rig
bpy.context.scene.SceneProp.panel_appearance_enabled = True
assert shape_transfer.apply(bpy.context, payload) is rig
assert rig.ShapeDrivers.male_80
definition = rig.ShapeDrivers.DRIVERS["height_33"][0]
expected = (payload["params"]["33"] - definition["value_min"]) * 100 / (definition["value_max"] - definition["value_min"])
assert abs(rig.ShapeValues.height_33 - expected) < 0.001
assert rig.mode == "POSE"
assert (rig.data.bones["mHead"].head_local - head).length > 0.01
assert before == {bone.name: [(con.name, con.type) for con in bone.constraints] for bone in rig.pose.bones}
assert len([obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE" and "avastar" in obj]) == 1
assert rig["aurora_shape_id"] == payload["id"]
bpy.ops.object.mode_set(mode="OBJECT")
meshes = [obj for obj in bpy.context.scene.objects if obj.type == "MESH" and obj.find_armature() == rig]
assert exporter.snapshot(meshes, set(payload["names"]), 1)["meshes"]
print("AURORA_NATIVE_AVASTAR_SHAPE_APPLY_OK", flush=True)
