"""Exercise list operators through the real client against an offline Aurora.

Run in Blender --background --factory-startup --python-exit-code 1 --python
this_file -- --port 17655 [--save-blend /absolute/path.blend].
"""
import argparse
import base64
from pathlib import Path
from queue import Empty, Queue
import sys
import time
import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import aurora_live
from aurora_live.client import Client

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, default=17655)
parser.add_argument("--save-blend", type=Path)
args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else [])
replies, sent = Queue(), []


class ObservedClient(Client):
    @staticmethod
    def _line(stream):
        reply = Client._line(stream)
        replies.put(reply)
        return reply

    def submit(self, value):
        sent.append(value)
        super().submit(value)


def flush(label, count, settle=1.0):
    before = len(sent)
    deadline = time.monotonic() + 10
    acknowledged = False
    while time.monotonic() < deadline:
        aurora_live._tick()
        try:
            while True:
                reply = replies.get_nowait()
                assert reply.get("ok", True), reply
                if len(sent) > before and reply.get("revision") == sent[-1]["revision"]:
                    acknowledged = True
        except Empty:
            pass
        if acknowledged:
            break
        time.sleep(0.02)
    assert acknowledged, (label, aurora_live._client.state())
    assert len(sent[-1]["meshes"]) == count, (label, sent[-1])
    print("AURORA_TRACKING_PHASE={} REVISION={} MESHES={} IMAGES={}".format(
        label, sent[-1]["revision"], count, len(sent[-1]["images"])), flush=True)
    time.sleep(settle)


aurora_live.register()
aurora_live.Client = ObservedClient
try:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    bpy.ops.aurora.create_sl_rig(with_tshirt=True)
    shirt = bpy.context.object
    material = shirt.data.materials[0]
    shader = next(n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    image = bpy.data.images.new("Live list paint", width=2, height=2)
    image.pixels.foreach_set([1, 0, 0, 1] * 4)
    image.update()
    texture = material.node_tree.nodes.new("ShaderNodeTexImage")
    texture.image = image
    material.node_tree.links.new(texture.outputs["Color"], shader.inputs["Base Color"])
    other = shirt.copy()
    other.data = shirt.data.copy()
    other.name = "Second listed shirt"
    bpy.context.collection.objects.link(other)
    blue = bpy.data.materials.new("Other shirt blue")
    blue.diffuse_color = (0.1, 0.2, 0.7, 1)
    other.data.materials.clear()
    other.data.materials.append(blue)
    other.select_set(True)
    bpy.ops.aurora.live_selection()
    entries = bpy.context.scene.aurora_live.objects
    assert len(entries) == 2
    bpy.context.scene.aurora_live.port = args.port
    bpy.ops.aurora.live_connect()
    flush("initial", 2, 4.5)
    broken = bpy.data.materials.new("Unsupported live material")
    broken.use_nodes = True
    output = next(node for node in broken.node_tree.nodes if node.type == "OUTPUT_MATERIAL")
    diffuse = broken.node_tree.nodes.new("ShaderNodeBsdfDiffuse")
    broken.node_tree.links.new(diffuse.outputs[0], output.inputs["Surface"])
    other.data.materials[0] = broken
    flush("invalid_source_isolated", 1)
    assert other.name in aurora_live._error and entries[1].error
    shirt.data.vertices[0].co.x += 0.1
    shirt.data.update()
    aurora_live._dirty = True
    flush("valid_source_still_updates", 1)
    other.data.materials[0] = blue
    flush("repaired_source_returns", 2)
    assert not aurora_live._error and not entries[1].error
    entries[0].enabled = False
    flush("pause_first", 1)
    image.pixels.foreach_set([0, 1, 0, 1] * 4)
    image.update()
    shirt.data.vertices[0].co.x += 0.08
    entries[1].enabled = False
    flush("pause_all", 0, 4.5)
    entries[0].enabled = True
    flush("resume_latest", 1)
    assert base64.b64decode(sent[-1]["images"][0]["rgba"])[:3] == b"\x00\xff\x00"
    bpy.ops.aurora.live_remove(index=0)
    flush("remove", 0)
    assert shirt.name in bpy.context.scene.objects
    other.select_set(False)
    shirt.select_set(True)
    bpy.ops.aurora.live_selection()
    flush("add_again", 1)
    entries[0].enabled = True
    flush("resume_other", 2)
    entries[0].enabled = False
    flush("final_pause", 1)
    bpy.ops.aurora.live_disconnect()
    assert len(entries) == 2 and not entries[0].enabled and entries[1].enabled
    for obj in bpy.context.selected_objects:
        obj.select_set(False)
    bpy.ops.aurora.live_connect()
    flush("reconnect_list", 1, 4.5)
    bpy.ops.aurora.live_disconnect()
    if args.save_blend:
        image.pack()
        bpy.ops.wm.save_as_mainfile(filepath=str(args.save_blend.resolve()))
finally:
    aurora_live.unregister()
print("AURORA_BLENDER_TRACKING_BRIDGE_OK", flush=True)
