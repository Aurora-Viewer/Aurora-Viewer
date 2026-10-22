"""Exercise a running offline Aurora bridge from real Blender.

Run Blender --background --factory-startup --python-exit-code 1
--python blender/tests/bridge_smoke.py -- --port 17655
"""
import argparse
import copy
import json
from pathlib import Path
import socket
import struct
import sys
import time

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from aurora_live import exporter
from aurora_live.client import Client, MAGIC

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, default=17655)
args = parser.parse_args(sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else [])
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
data = bpy.data.armatures.new("Live smoke rig")
rig = bpy.data.objects.new("Live smoke rig", data)
bpy.context.collection.objects.link(rig)
bpy.context.view_layer.objects.active = rig
rig.select_set(True)
bpy.ops.object.mode_set(mode="EDIT")
bone = data.edit_bones.new("mChest")
bone.head, bone.tail = (0,0,1), (0,0,1.3)
bpy.ops.object.mode_set(mode="OBJECT")
bpy.ops.mesh.primitive_uv_sphere_add(segments=24, ring_count=12, radius=0.3, location=(0,0,1))
mesh = bpy.context.object
mesh.name = "Blender live smoke shell"
mesh.vertex_groups.new(name="mChest").add(list(range(len(mesh.data.vertices))), 1.0, "REPLACE")
mesh.modifiers.new("Armature", "ARMATURE").object = rig
material = bpy.data.materials.new("Live green")
material.diffuse_color = (0.1,0.8,0.3,1.0)
mesh.data.materials.append(material)
bpy.context.view_layer.update()

connection = None
deadline = time.monotonic() + 45
while connection is None:
    try:
        connection = socket.create_connection(("127.0.0.1", args.port), timeout=2)
    except OSError:
        if time.monotonic() >= deadline:
            raise
        time.sleep(0.2)

with connection:
    connection.settimeout(10)
    connection.sendall(MAGIC)
    reader = connection.makefile("rb")
    hello = Client._line(reader)
    supported = set(hello["joints"])
    assert hello["protocol"] == 1 and "mChest" in supported

    def send(value, expected=True):
        started = time.perf_counter()
        payload = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
        connection.sendall(struct.pack("<I", len(payload)) + payload)
        reply = Client._line(reader)
        assert reply["ok"] is expected, reply
        milliseconds = (time.perf_counter() - started) * 1000
        print("AURORA_BRIDGE_REVISION={} OK={} BYTES={} ACK_MS={:.1f}".format(value["revision"], expected, len(payload), milliseconds), flush=True)
        time.sleep(0.4)

    first = exporter.snapshot([mesh], supported, 1, rest_axes=hello.get("rest_axes"))
    send(first)
    mesh.data.vertices[0].co.x += 0.12
    mesh.data.update()
    second = exporter.snapshot([mesh], supported, 2, rest_axes=hello.get("rest_axes"))
    send(second)
    invalid = copy.deepcopy(second)
    invalid["revision"] = 3
    invalid["meshes"][0]["faces"][0]["vertices"][0]["weights"] = [0.0]*4
    send(invalid, False)
    send({"protocol":1,"revision":4,"meshes":[]})
    send(exporter.snapshot([mesh], supported, 5, rest_axes=hello.get("rest_axes")))
    reader.close()
print("AURORA_BLENDER_BRIDGE_SMOKE_OK", flush=True)
