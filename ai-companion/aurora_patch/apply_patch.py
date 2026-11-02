"""Install the opt-in Rust AI bridge in an Aurora checkout (no remote writes)."""
from pathlib import Path
import argparse
import sys

parser = argparse.ArgumentParser()
parser.add_argument("checkout", type=Path)
args = parser.parse_args()
root = args.checkout.resolve()
app = root / "crates/aurora-viewer/src/app.rs"
target = root / "crates/aurora-viewer/src/app/ai_control.rs"
if not all(path.exists() for path in (root / "Cargo.toml", app)) or not target.parent.is_dir():
    sys.exit("Not an Aurora Viewer checkout")
if target.exists():
    sys.exit("Bridge source already exists. Refusing to overwrite it.")
original = app.read_text(encoding="utf-8")
revised = original
edits = [
    ("mod about;\n", "mod about;\nmod ai_control;\n"),
    ("    world: World,\n", "    world: World,\n    ai_bridge: Option<ai_control::Bridge>,\n"),
    ("            bridge_runtime: Default::default(),\n",
     "            ai_bridge: ai_control::Bridge::start(),\n            bridge_runtime: Default::default(),\n"),
    ("        self.update_move_input(typing, dt);\n",
     "        self.update_move_input(typing, dt);\n        self.poll_ai_bridge();\n"),
    ("        self.update_autopilot(dt);\n",
     "        self.update_autopilot(dt);\n        // Apply AI movement to the REAL avatar controls, not animation-only inputs.\n"
     "        if let Some(bridge) = self.ai_bridge.as_ref() {\n"
     "            bridge.apply_move_input(&mut self.input);\n"
     "        }\n"),
]
for before, after in edits:
    count = revised.count(before)
    if count != 1:
        sys.exit(f"Source changed: expected one occurrence of {before!r}, got {count}. No changes made.")
    revised = revised.replace(before, after, 1)
backup = app.with_suffix(".rs.ai-backup")
backup.write_bytes(app.read_bytes())
app.write_text(revised, encoding="utf-8")
target.write_bytes((Path(__file__).parent / "ai_control.rs").read_bytes())
print("Patched Rust viewer source. Rust compilation/live-grid validation still required.")
