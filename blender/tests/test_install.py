"""Install/enable/disable either ZIP layout in an isolated Blender profile.

Set BLENDER_USER_EXTENSIONS to a temporary directory before starting Blender.
Run with --background --factory-startup --python-exit-code 1 --python this_file
-- /absolute/path/to/aurora-live-mesh.zip.
"""
import os
from pathlib import Path
import sys
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
assert bpy.ops.aurora.create_sl_rig(with_tshirt=True) == {"FINISHED"}
shirt = bpy.context.object
assert len(shirt.find_armature().data.bones) == 159
assert bpy.ops.aurora.auto_weights() == {"FINISHED"}
result = bpy.ops.preferences.addon_disable(module=module)
assert "FINISHED" in result and not hasattr(bpy.types.Scene, "aurora_live"), result
print("AURORA_BLENDER_INSTALL_OK", flush=True)
