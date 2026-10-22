"""Build a Blender 4.3+ extension or 2.93+ add-on using Python's stdlib."""
import argparse
from pathlib import Path
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("output", type=Path, help="Destination ZIP file")
parser.add_argument("--legacy", action="store_true", help="Use the add-on layout for Blender 2.93–4.2")
args = parser.parse_args()
source = Path(__file__).resolve().parent / "aurora_live"
args.output.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(args.output, "w", zipfile.ZIP_DEFLATED) as archive:
    for name in ("__init__.py", "client.py", "exporter.py", "blender_manifest.toml"):
        archive.write(source / name, "aurora_live/" + name if args.legacy else name)
    archive.write(source.parents[1] / "LICENSE", "aurora_live/LICENSE" if args.legacy else "LICENSE")
print(args.output.resolve())
