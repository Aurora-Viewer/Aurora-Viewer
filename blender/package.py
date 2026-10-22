"""Build the Blender 4.3+ extension using Python's stdlib."""
import argparse
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=Path, help="Destination ZIP file")
args = parser.parse_args()
source = Path(__file__).resolve().parent / "aurora_live"
args.output.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED) as archive:
    for name in ("__init__.py", "client.py", "exporter.py", "starter.py", "SKELETON-NOTICE.txt", "blender_manifest.toml"):
        archive.write(source / name, name)
    archive.write(source.parents[1] / "crates/aurora-viewer/assets/character/avatar_skeleton.xml", "avatar_skeleton.xml")
    archive.write(source.parents[1] / "LICENSE", "LICENSE")
print(args.output.resolve())
