"""Install/enable/disable the extension ZIP in an isolated Blender 4.3+ profile.

Set BLENDER_USER_EXTENSIONS to a temporary directory before starting Blender.
Run with --background --factory-startup --python-exit-code 1 --python this_file
-- /absolute/path/to/aurora-live-mesh.zip.
"""
import os
from pathlib import Path
import sys
import bpy

if not os.environ.get("BLENDER_USER_EXTENSIONS"):
    raise RuntimeError("Set BLENDER_USER_EXTENSIONS to an isolated test directory")
args = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else []
if len(args) != 1:
    raise ValueError("Pass the add-on ZIP after --")
path = Path(args[0]).resolve()
assert path.is_file(), path
result = bpy.ops.extensions.package_install_files(filepath=str(path), repo="user_default", enable_on_install=True)
assert "FINISHED" in result, result
assert hasattr(bpy.types.Scene, "aurora_live"), "extension was not enabled"
assert bpy.ops.aurora.create_sl_rig(with_tshirt=True) == {"FINISHED"}
shirt = bpy.context.object
assert len(shirt.find_armature().data.bones) == 159
assert bpy.ops.aurora.auto_weights() == {"FINISHED"}
result = bpy.ops.preferences.addon_disable(module="bl_ext.user_default.aurora_live_mesh")
assert "FINISHED" in result and not hasattr(bpy.types.Scene, "aurora_live"), result
print("AURORA_BLENDER_INSTALL_OK", flush=True)
