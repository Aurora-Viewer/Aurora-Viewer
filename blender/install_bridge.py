"""Aurora's bundled, offline installer, run by the selected Blender executable.

Blender loads its preferences normally (re-reading them via an operator crashes
some 4.2 builds). Only Aurora's package/enable state is changed, and no scene is
saved. Probe never saves preferences.
"""
import ast
from contextlib import contextmanager
from datetime import datetime, timezone
import importlib
import json
import os
from pathlib import Path
import re
import shutil
import sys
import uuid
import zipfile

import bpy

# Background probes must not autosave preferences dirtied by existing add-ons.
# Installation serializes the original value, then disables autosave again for
# this disposable process, including error/rollback exits.
AUTO_SAVE = getattr(bpy.context.preferences, "use_preferences_save", None)
if AUTO_SAVE is not None:
    bpy.context.preferences.use_preferences_save = False


def checked(result):
    if "FINISHED" not in result:
        raise RuntimeError("Blender cancelled the operation: " + str(result))


def package_version(directory, extension):
    try:
        if extension:
            text = (directory / "blender_manifest.toml").read_text(encoding="utf-8")
            match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
            return match.group(1) if match else None
        tree = ast.parse((directory / "__init__.py").read_text(encoding="utf-8"))
        for node in tree.body:
            if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "bl_info" for t in node.targets):
                return ".".join(str(v) for v in ast.literal_eval(node.value)["version"])
    except (OSError, SyntaxError, ValueError, KeyError, TypeError):
        pass
    return None


def locations():
    extension = bpy.app.version >= (4, 2, 0)
    config = Path(bpy.utils.user_resource("CONFIG"))
    overridden = os.environ.get("BLENDER_USER_CONFIG")
    if overridden and config.resolve() != Path(overridden).resolve():
        raise RuntimeError("Blender did not honor BLENDER_USER_CONFIG; refusing to touch a different profile.")
    legacy = Path(bpy.utils.user_resource("SCRIPTS")) / "addons" / "aurora_live"
    repo = None
    if extension:
        repos = bpy.context.preferences.extensions.repos
        # Update an existing local installation in place, including a custom repo.
        repo = next((r for r in repos if r.enabled and not r.use_remote_url
                     and (Path(r.directory) / "aurora_live_mesh").is_dir()), None)
        if repo is None:
            repo = next((r for r in repos if r.module == "user_default" and r.enabled and not r.use_remote_url), None)
        if repo is None:
            raise RuntimeError("No enabled local extension repository. Enable User Default in Blender preferences.")
        target = Path(repo.directory) / "aurora_live_mesh"
        module = "bl_ext." + repo.module + ".aurora_live_mesh"
    else:
        target, module = legacy, "aurora_live"
    return extension, config, legacy, repo, target, module


@contextmanager
def install_lock(path):
    """The OS releases this lock even if the viewer terminates its child."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a+b") as handle:
        handle.seek(0, os.SEEK_END)
        if handle.tell() == 0:
            handle.write(b"0")
            handle.flush()
        handle.seek(0)
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            yield
        finally:
            if os.name == "nt":
                handle.seek(0)
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)


def install(request, state):
    extension, config, legacy, repo, target, module = state
    expected = request["plugin_version"]
    old = package_version(target, extension) or (package_version(legacy, False) if extension else None)
    if old and tuple(int(v) for v in old.split(".")) > tuple(int(v) for v in expected.split(".")):
        raise RuntimeError("A newer Aurora plug-in is already installed; downgrade refused.")
    package = Path(request["extension_zip"] if extension else request["legacy_zip"])
    with zipfile.ZipFile(package) as archive:
        files = {}
        for name in archive.namelist():
            relative = Path(name if extension else name.removeprefix("aurora_live/"))
            if relative.is_absolute() or ".." in relative.parts:
                raise RuntimeError("Invalid path in the bundled Aurora package.")
            if not name.endswith("/"):
                files[relative] = archive.read(name)
    if Path("__init__.py") not in files:
        raise RuntimeError("The bundled Aurora package is incomplete.")
    config.parent.mkdir(parents=True, exist_ok=True)
    config.mkdir(parents=True, exist_ok=True)
    backup = config.parent / "aurora-backups" / (datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ-") + uuid.uuid4().hex)
    backup.mkdir(parents=True)
    prefs = config / "userpref.blend"
    before_addons = set(bpy.context.preferences.addons.keys())
    directories = [(target, backup / "package")]
    if extension and legacy != target:
        directories.append((legacy, backup / "legacy"))
    # Resolve every path once; rollback only touches these exact owned targets.
    existed = []
    for path, saved in directories:
        if path.is_symlink() or (path.exists() and not path.is_dir()):
            raise RuntimeError("Aurora package location is not a regular directory: " + str(path))
        existed.append(path.is_dir())
        if path.is_dir():
            shutil.copytree(path, saved, symlinks=True)
    if prefs.is_file():
        shutil.copy2(prefs, backup / "userpref.blend")
    directories = [(path.resolve(), saved) for path, saved in directories]
    try:
        if extension and "aurora_live" in before_addons:
            checked(bpy.ops.preferences.addon_disable(module="aurora_live"))
        if extension:
            checked(bpy.ops.extensions.package_install_files(
                filepath=request["extension_zip"], repo=repo.module, enable_on_install=True, overwrite=True))
        else:
            checked(bpy.ops.preferences.addon_install(filepath=request["legacy_zip"], overwrite=True))
            checked(bpy.ops.preferences.addon_enable(module=module))
        installed = importlib.import_module(module)
        if package_version(Path(installed.__file__).parent, extension) != expected:
            raise RuntimeError("Installed Aurora plug-in version does not match the bundled package.")
        # Some Blender operators return FINISHED after reporting an install error.
        # A previously enabled package must never be mistaken for a new install.
        for relative, contents in files.items():
            if (target / relative).read_bytes() != contents:
                raise RuntimeError("Blender did not install the complete bundled Aurora package: " + str(relative))
        if not hasattr(bpy.types.Scene, "aurora_live") or module not in bpy.context.preferences.addons:
            raise RuntimeError("Aurora plug-in did not enable correctly.")
        after_addons = set(bpy.context.preferences.addons.keys())
        if after_addons - {module, "aurora_live"} != before_addons - {module, "aurora_live"}:
            raise RuntimeError("Unrelated enabled add-ons changed; restoring the previous installation.")
        # Old Blender falls back to its normal config if an environment override
        # points to a directory that does not exist at save time.
        config.mkdir(parents=True, exist_ok=True)
        try:
            if AUTO_SAVE is not None:
                bpy.context.preferences.use_preferences_save = AUTO_SAVE
            checked(bpy.ops.wm.save_userpref())
        finally:
            if AUTO_SAVE is not None:
                bpy.context.preferences.use_preferences_save = False
        if not prefs.is_file():
            raise RuntimeError("Blender did not save preferences in the selected profile.")
    except Exception as error:
        # Preserve the backup even after a successful rollback. Never touch scenes.
        try:
            for (path, saved), was_present in zip(directories, existed):
                if path.is_dir():
                    shutil.rmtree(path)
                if was_present:
                    shutil.copytree(saved, path, symlinks=True)
            config.mkdir(parents=True, exist_ok=True)
            if (backup / "userpref.blend").is_file():
                shutil.copy2(backup / "userpref.blend", prefs)
            elif prefs.is_file():
                prefs.unlink()
        except Exception as rollback_error:
            raise RuntimeError(str(error) + "; restore backup from " + str(backup) + ": " + str(rollback_error)) from error
        raise RuntimeError(str(error) + "; previous files restored; backup: " + str(backup)) from error
    return str(backup)


def run(request):
    report = {"version": list(bpy.app.version[:3]), "plugin_version": None, "enabled": False,
              "profile": "", "target": "", "backup": None, "error": None}
    if bpy.app.version < (2, 93, 0):
        return report
    state = locations()
    extension, config, legacy, repo, target, module = state
    report["profile"], report["target"] = str(config.parent), str(target)
    report["plugin_version"] = package_version(target, extension) or (package_version(legacy, False) if extension else None)
    report["enabled"] = module in bpy.context.preferences.addons
    if request["operation"] == "install":
        with install_lock(config.parent / ".aurora-install.lock"):
            report["backup"] = install(request, state)
            report["plugin_version"], report["enabled"] = request["plugin_version"], True
    return report


if __name__ == "__main__":
    arguments = sys.argv[sys.argv.index("--") + 1:]
    request_file, report_file = map(Path, arguments)
    try:
        result = run(json.loads(request_file.read_text(encoding="utf-8")))
    except Exception as error:
        result = {"version": list(bpy.app.version[:3]), "error": str(error)}
    report_file.write_text(json.dumps(result, ensure_ascii=False), encoding="utf-8")
    if result.get("error"):
        raise RuntimeError(result["error"])
