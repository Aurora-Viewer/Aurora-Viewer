"""Build a Blender 4.2+ extension or 2.93+ legacy add-on from shared sources."""
import argparse
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=Path, help="Destination ZIP file")
parser.add_argument("--legacy", action="store_true", help="Classic add-on layout for Blender 2.93–4.1")
args = parser.parse_args()
source = Path(__file__).resolve().parent / "aurora_live"
args.output.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED) as archive:
    prefix = "aurora_live/" if args.legacy else ""
    for name in ("__init__.py", "compat.py", "client.py", "exporter.py", "starter.py", "weights.py", "materials.py", "tracking.py",
                 "handles.py", "rigging.py", "skinning.py", "posing.py", "tools_ui.py", "lods.py", "shape_apply.py", "shape_transfer.py", "SKELETON-NOTICE.txt"):
        archive.write(source / name, prefix + name)
    if not args.legacy:
        archive.write(source / "blender_manifest.toml", "blender_manifest.toml")
    archive.write(source.parents[1] / "crates/aurora-viewer/assets/character/avatar_skeleton.xml", prefix + "avatar_skeleton.xml")
    archive.write(source.parents[1] / "LICENSE", prefix + "LICENSE")
print(args.output.resolve())
