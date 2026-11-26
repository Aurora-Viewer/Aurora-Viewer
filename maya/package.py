"""Build the Maya plug-in ZIP from the sources (the viewer bundles the same
files: crates/aurora-viewer/src/blender_live/addon.rs)."""
import argparse
from pathlib import Path
import zipfile

FILES = ("__init__.py", "aurora_live_command.py", "bindinfo.py", "build.py", "capture.py", "controls.py", "convert.py", "lods.py", "materials.py", "mayastar.py", "rig.py", "rig_rules.py", "rigs.py", "session.py", "shape.py", "shape_apply.py", "shape_rules.py", "ui.py", "weights.py")

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=Path, help="Destination ZIP file")
args = parser.parse_args()
root = Path(__file__).resolve().parent
args.output.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED) as archive:
    archive.write(root / "install.mel", "install.mel")
    archive.write(root / "aurora_live_maya_install.py", "aurora_live_maya_install.py")
    for name in FILES:
        archive.write(root / "aurora_live_maya" / name, "aurora_live_maya/" + name)
    # The transport is shared with the Blender plug-in; it never imports bpy.
    archive.write(root.parent / "blender/aurora_live/client.py", "aurora_live_maya/client.py")
    # The SL skeleton of the shape transfer, with its notice (as the Blender ZIP).
    archive.write(root.parent / "crates/aurora-viewer/assets/character/avatar_skeleton.xml",
                  "aurora_live_maya/avatar_skeleton.xml")
    archive.write(root.parent / "blender/aurora_live/SKELETON-NOTICE.txt", "aurora_live_maya/SKELETON-NOTICE.txt")
    archive.write(root.parent / "LICENSE", "LICENSE")
print(args.output.resolve())
