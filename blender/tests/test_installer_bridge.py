"""Exercise the viewer's installer in disposable real Blender profiles.

Run with system Python: test_installer_bridge.py /absolute/path/to/blender
--extension-zip <package> --legacy-zip <package> --work <temporary-directory>.
Never uses the caller's Blender profile or loads a scene.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import zipfile


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("blender", type=Path)
parser.add_argument("--extension-zip", required=True, type=Path)
parser.add_argument("--legacy-zip", required=True, type=Path)
parser.add_argument("--work", required=True, type=Path)
args = parser.parse_args()
root = args.work.resolve()
root.mkdir(parents=True, exist_ok=False)
bridge = Path(__file__).resolve().parents[1] / "install_bridge.py"
version = re.search(r'^version = "([^"]+)"', (bridge.parent / "aurora_live/blender_manifest.toml").read_text(), re.MULTILINE).group(1)
major, minor, patch = map(int, version.split("."))
previous = ".".join(map(str, (major, minor, patch - 1) if patch else
                        (major, minor - 1, 0) if minor else (major - 1, 0, 0)))
environment = dict(os.environ)
(root / "tmp").mkdir()
environment["TEMP"] = environment["TMP"] = str(root / "tmp")
for kind, path in {"CONFIG": "config", "SCRIPTS": "scripts", "EXTENSIONS": "extensions", "RESOURCES": ""}.items():
    environment["BLENDER_USER_" + kind] = str(root / "profile" / path)
    (root / "profile" / path).mkdir(parents=True, exist_ok=True)
base = [str(args.blender.resolve()), "--background", "--disable-autoexec", "--python-exit-code", "1"]


def invoke(operation, expected=version, extension=None, legacy=None, good=True):
    request = {"operation": operation, "plugin_version": expected,
               "extension_zip": str((extension or args.extension_zip).resolve()),
               "legacy_zip": str((legacy or args.legacy_zip).resolve())}
    (root / "request.json").write_text(json.dumps(request), encoding="utf-8")
    output = root / "report.json"
    output.unlink(missing_ok=True)
    result = subprocess.run(base + ["--python", str(bridge), "--", str(root / "request.json"), str(output)],
                            env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
    (root / (operation + ".log")).write_bytes(result.stdout)
    report = json.loads(output.read_text(encoding="utf-8"))
    assert bool(result.returncode == 0) == good, report
    assert bool(report.get("error") is None) == good, report
    return report


def script(source):
    file = root / "fixture.py"
    file.write_text(source, encoding="utf-8")
    result = subprocess.run(base + ["--python", str(file)], env=environment,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
    (root / "fixture.log").write_bytes(result.stdout)
    assert result.returncode == 0, result.stdout.decode("utf-8", errors="replace")


def older_zip(source, destination):
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in original.namelist():
            contents = original.read(name)
            if name.endswith("blender_manifest.toml"):
                contents = contents.replace(('version = "' + version + '"').encode(), ('version = "' + previous + '"').encode())
            if name.endswith("__init__.py"):
                contents = contents.replace(('"version": (' + version.replace('.', ', ') + ')').encode(),
                                            ('"version": (' + previous.replace('.', ', ') + ')').encode())
            archive.writestr(name, contents)


initial = invoke("probe")
assert not initial["enabled"] and initial["plugin_version"] is None
assert not (root / "profile/config/userpref.blend").exists(), "probe must not save preferences"

# Create an unrelated enabled add-on and non-default preference before installing.
script('''
import bpy
import os
from pathlib import Path
assert Path(bpy.utils.user_resource('CONFIG')).resolve() == Path(os.environ['BLENDER_USER_CONFIG']).resolve()
addons = Path(bpy.utils.user_resource('SCRIPTS', create=True)) / 'addons'
addons.mkdir(parents=True, exist_ok=True)
(addons / 'aurora_installer_test_guard.py').write_text('bl_info={"name":"Installer test guard","blender":(2,93,0)}\\ndef register(): pass\\ndef unregister(): pass\\n')
bpy.utils.refresh_script_paths()
assert bpy.ops.preferences.addon_enable(module='aurora_installer_test_guard') == {'FINISHED'}
bpy.context.preferences.inputs.use_emulate_numpad = True
assert bpy.ops.wm.save_userpref() == {'FINISHED'}
''')
old_extension, old_legacy = root / "older-extension.zip", root / "older-legacy.zip"
older_zip(args.extension_zip, old_extension)
older_zip(args.legacy_zip, old_legacy)
old = invoke("install", previous, old_extension, old_legacy)
assert old["enabled"] and old["plugin_version"] == previous
marker = Path(old["target"]) / "previous-marker.txt"
marker.write_text("previous Aurora package", encoding="utf-8")
updated = invoke("install")
assert updated["enabled"] and updated["plugin_version"] == version
backup = Path(updated["backup"])
assert (backup / "package/previous-marker.txt").read_text() == "previous Aurora package"
assert (backup / "userpref.blend").is_file()
assert invoke("probe")["enabled"], "activation persists across Blender restarts"

script('''
import bpy
module = 'aurora_live' if bpy.app.version < (4, 2, 0) else 'bl_ext.user_default.aurora_live_mesh'
assert bpy.ops.preferences.addon_disable(module=module) == {'FINISHED'}
assert bpy.ops.wm.save_userpref() == {'FINISHED'}
''')
assert not invoke("probe")["enabled"]
assert invoke("install")["enabled"], "the Enable action also repairs the bundled package"

# A failed native installation restores the package and preferences.
target = Path(updated["target"])
source_before = (target / "__init__.py").read_bytes()
preferences = root / "profile/config/userpref.blend"
prefs_before = preferences.read_bytes()
invoke("install", extension=root / "missing.zip", legacy=root / "missing.zip", good=False)
assert (target / "__init__.py").read_bytes() == source_before
assert preferences.read_bytes() == prefs_before
assert invoke("probe")["enabled"]

# Force a failure after native replacement has begun, testing the rollback.
broken = []
for index, source in enumerate((args.extension_zip, args.legacy_zip)):
    destination = root / ("broken-" + str(index) + ".zip")
    with zipfile.ZipFile(source) as original, zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as archive:
        for name in original.namelist():
            contents = b"def broken(:\n" if name.endswith("__init__.py") else original.read(name)
            archive.writestr(name, contents)
    broken.append(destination)
invoke("install", extension=broken[0], legacy=broken[1], good=False)
assert (target / "__init__.py").read_bytes() == source_before
assert preferences.read_bytes() == prefs_before
assert invoke("probe")["enabled"]

# Refuse a downgrade, even if called outside the UI.
invoke("install", previous, old_extension, old_legacy, good=False)
assert (target / "__init__.py").read_bytes() == source_before
assert preferences.read_bytes() == prefs_before
if initial["version"] >= [4, 2, 0]:
    script('''
import bpy
import shutil
from pathlib import Path
assert bpy.ops.preferences.addon_disable(module='bl_ext.user_default.aurora_live_mesh') == {'FINISHED'}
repo = next(r for r in bpy.context.preferences.extensions.repos if r.module == 'user_default')
shutil.rmtree(Path(repo.directory) / 'aurora_live_mesh')
assert bpy.ops.preferences.addon_install(filepath=LEGACY_ZIP, overwrite=True) == {'FINISHED'}
assert bpy.ops.preferences.addon_enable(module='aurora_live') == {'FINISHED'}
assert bpy.ops.wm.save_userpref() == {'FINISHED'}
'''.replace('LEGACY_ZIP', repr(str(old_legacy))))
    assert invoke("probe")["plugin_version"] == previous
    assert invoke("install")["enabled"]
script('''
import bpy
assert 'aurora_installer_test_guard' in bpy.context.preferences.addons
assert bpy.context.preferences.inputs.use_emulate_numpad
assert 'aurora_live' in bpy.context.preferences.addons or any(k.endswith('.aurora_live_mesh') for k in bpy.context.preferences.addons.keys())
if bpy.app.version >= (4, 2, 0):
    assert 'aurora_live' not in bpy.context.preferences.addons, 'classic and extension must not be enabled together'
''')
print("PASS native installer", initial["version"], "fresh install, upgrade, enable, backup, rollback, downgrade refusal, unrelated preferences, classic migration")
