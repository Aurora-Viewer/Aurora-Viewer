"""Install 0.2.0 then upgrade in the same Blender process to catch stale imports.

Use an isolated BLENDER_USER_EXTENSIONS/CONFIG/SCRIPTS and pass the old and
new extension ZIP paths after --.
"""
import importlib
import os
from pathlib import Path
import sys
import bpy

if not os.environ.get("BLENDER_USER_EXTENSIONS"):
    raise RuntimeError("An isolated extension directory is required")
paths = [Path(value).resolve() for value in sys.argv[sys.argv.index("--")+1:]]
assert len(paths) == 2 and all(path.is_file() for path in paths)
module_name = "bl_ext.user_default.aurora_live_mesh"
for index, path in enumerate(paths):
    assert bpy.ops.extensions.package_install_files(filepath=str(path), repo="user_default", enable_on_install=True) == {"FINISHED"}
    package = importlib.import_module(module_name)
    manifest = Path(package.__file__).with_name("blender_manifest.toml").read_text(encoding="utf-8")
    if index == 0:
        assert 'version = "0.2.0"' in manifest
        assert bpy.ops.aurora.create_sl_rig(with_tshirt=True) == {"FINISHED"}
        shirt = bpy.context.object
        names = {bone.name for bone in shirt.find_armature().data.bones}
        assert "images" not in package.exporter.snapshot([shirt], names, 1)
    else:
        assert 'version = "0.5.0"' in manifest
        snapshot = package.exporter.snapshot([shirt], names, 2)
        assert "images" in snapshot and "material" in snapshot["meshes"][0]["faces"][0]
        assert hasattr(package, "materials") and hasattr(package, "weights")
        assert bpy.ops.aurora.auto_weights() == {"FINISHED"}
assert bpy.ops.preferences.addon_disable(module=module_name) == {"FINISHED"}
print("AURORA_BLENDER_UPGRADE_OK", flush=True)
