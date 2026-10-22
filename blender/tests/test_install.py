"""Install/enable/disable the older add-on ZIP in an isolated test profile.

Set BLENDER_USER_SCRIPTS to a temporary directory before starting Blender.
Run with --background --factory-startup --python-exit-code 1 --python this_file
-- /absolute/path/to/aurora-live-mesh-legacy.zip.
"""
import os
from pathlib import Path
import sys
import bpy

if not os.environ.get("BLENDER_USER_SCRIPTS"):
    raise RuntimeError("Set BLENDER_USER_SCRIPTS to an isolated test directory")
args = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else []
if len(args) != 1:
    raise ValueError("Pass the add-on ZIP after --")
path = Path(args[0]).resolve()
assert path.is_file(), path
result = bpy.ops.preferences.addon_install(filepath=str(path), overwrite=True)
assert "FINISHED" in result, result
result = bpy.ops.preferences.addon_enable(module="aurora_live")
assert "FINISHED" in result and hasattr(bpy.types.Scene, "aurora_live"), result
result = bpy.ops.preferences.addon_disable(module="aurora_live")
assert "FINISHED" in result and not hasattr(bpy.types.Scene, "aurora_live"), result
print("AURORA_BLENDER_INSTALL_OK", flush=True)
