"""Exercise the real Blender client against Aurora's offline shape demo."""
import argparse
from pathlib import Path
import sys
import time

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live import tracking, starter

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, default=17655)
parser.add_argument("--readonly", action="store_true")
parser.add_argument("--apply", action="store_true")
parser.add_argument("--save-blend")
parser.add_argument("--hold", type=float, default=1, help="Keep the source connected for an offline viewer capture")
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
aurora_live.register()
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
target, shirt = (starter.create(bpy.context) if args.apply else (None, None))
if target:
    shirt.select_set(False)
    target.select_set(True)
    bpy.context.view_layer.objects.active = target
bpy.context.scene.aurora_live.port = args.port
assert bpy.ops.aurora.live_connect() == {"FINISHED"}
connection = aurora_live._client
try:
    deadline = time.monotonic() + (8 if args.readonly else 35)
    rig = None
    while time.monotonic() < deadline:
        aurora_live._tick()
        rig = next((obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE" and obj.get("aurora_shape_id")), None)
        if rig:
            break
        time.sleep(0.1)
    if args.readonly:
        assert rig is None, "a non-modifiable shape must never leave Aurora"
        assert connection.state()[1], "the denial test must have a real connected client"
        print("AURORA_SHAPE_READONLY_OK", flush=True)
    else:
        assert rig, "no shape received: " + connection.state()[0]
        assert len(rig.data.bones) == 159
        assert len(rig.children) == (1 if args.apply else 6)
        if args.apply:
            assert rig is target, "Apply must keep the selected rig instead of creating another"
            assert len([obj for obj in bpy.context.scene.objects if obj.type == "ARMATURE"]) == 1
        assert not bpy.context.scene.aurora_live.objects, "reference bodies are not added to the live list"
        upper = shirt if args.apply else next(obj for obj in rig.children if obj.name.startswith("Shape upper"))
        expected = "revision {}".format(aurora_live._revision + 1)
        tracking.add(bpy.context.scene, [upper])
        aurora_live._request_capture()
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            aurora_live._tick()
            if expected in connection.state()[0]:
                break
            time.sleep(0.1)
        assert not aurora_live._error, aurora_live._error
        assert expected in connection.state()[0], connection.state()[0]
        if args.save_blend:
            bpy.ops.wm.save_as_mainfile(filepath=str(Path(args.save_blend).resolve()))
        print("AURORA_SHAPE_ROUND_TRIP_OK", flush=True)
    deadline = time.monotonic() + max(0, min(args.hold, 60))
    while time.monotonic() < deadline:
        aurora_live._tick()
        time.sleep(0.1)
finally:
    aurora_live._stop()
    aurora_live.unregister()
