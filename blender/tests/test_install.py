"""Install/enable/disable either ZIP layout in an isolated Blender profile.

Set BLENDER_USER_EXTENSIONS to a temporary directory before starting Blender.
Run with --background --factory-startup --python-exit-code 1 --python this_file
-- /absolute/path/to/aurora-live-mesh.zip.
"""
import os
from pathlib import Path
import sys
import importlib
import zipfile
import bpy

args = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else []
if len(args) != 1:
    raise ValueError("Pass the add-on ZIP after --")
path = Path(args[0]).resolve()
assert path.is_file(), path
with zipfile.ZipFile(path) as archive:
    extension = "blender_manifest.toml" in archive.namelist()
if extension:
    if not os.environ.get("BLENDER_USER_EXTENSIONS"):
        raise RuntimeError("Set BLENDER_USER_EXTENSIONS to an isolated test directory")
    module = "bl_ext.user_default.aurora_live_mesh"
    result = bpy.ops.extensions.package_install_files(filepath=str(path), repo="user_default", enable_on_install=True)
else:
    if not os.environ.get("BLENDER_USER_SCRIPTS"):
        raise RuntimeError("Set BLENDER_USER_SCRIPTS to an isolated test directory")
    module = "aurora_live"
    result = bpy.ops.preferences.addon_install(filepath=str(path), overwrite=True)
    assert "FINISHED" in result, result
    result = bpy.ops.preferences.addon_enable(module=module)
assert "FINISHED" in result, result
assert hasattr(bpy.types.Scene, "aurora_live"), "extension was not enabled"
prefs = bpy.context.scene.aurora_live
assert prefs.auto_update and abs(prefs.interval - 2.0) < 0.000001
prefs.auto_update = False
prefs.interval = 45
assert prefs.interval == 45 and not bpy.ops.aurora.live_refresh.poll()
prefs.auto_update = True
installed = importlib.import_module(module)
if extension:
    # Blender strips bl_info from extension modules; its manifest is authoritative.
    assert 'version = "0.14.7"' in Path(installed.__file__).with_name("blender_manifest.toml").read_text()
else:
    assert installed.bl_info["version"] == (0, 14, 7)
assert bpy.ops.aurora.create_sl_rig(with_tshirt=True) == {"FINISHED"}
shirt = bpy.context.object
assert len(shirt.find_armature().data.bones) == 159
entries = bpy.context.scene.aurora_live.objects
assert len(entries) == 1 and entries[0].object == shirt and entries[0].enabled
assert bpy.ops.aurora.live_selection() == {"FINISHED"}
assert len(entries) == 1
entries[0].enabled = False
assert bpy.ops.aurora.auto_weights() == {"FINISHED"}
assert not entries[0].enabled
assert bpy.ops.aurora.live_remove(index=0) == {"FINISHED"}
assert not entries and shirt.name in bpy.context.scene.objects
assert bpy.ops.aurora.create_ik() == {"FINISHED"}
assert len(shirt.find_armature().data.bones) == 167
assert bpy.ops.aurora.switch_ik(limb="HAND_L", enabled=True) == {"FINISHED"}
assert bpy.ops.aurora.switch_ik(limb="HAND_L", enabled=False) == {"FINISHED"}
assert bpy.ops.aurora.bone_visibility(part="FACE", visible=True) == {"FINISHED"}
assert bpy.ops.aurora.transfer_weights(clean=True) == {"FINISHED"}
for kind in ("ROOT", "CLASSIC", "BENTO"):
    assert bpy.ops.aurora.create_pose_controls(kind=kind) == {"FINISHED"}
assert len(shirt.find_armature().data.bones) == 171
for kind in ("CLASSIC", "BENTO"):
    assert bpy.ops.aurora.switch_look_at(kind=kind, enabled=True) == {"FINISHED"}
    assert bpy.ops.aurora.center_look_at(kind=kind) == {"FINISHED"}
    assert bpy.ops.aurora.switch_look_at(kind=kind, enabled=False) == {"FINISHED"}
result = bpy.ops.preferences.addon_disable(module=module)
assert "FINISHED" in result and not hasattr(bpy.types.Scene, "aurora_live"), result
print("AURORA_BLENDER_INSTALL_OK", flush=True)
