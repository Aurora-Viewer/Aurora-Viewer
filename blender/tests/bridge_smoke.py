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
from aurora_live import exporter, starter, materials, weights
from aurora_live.client import Client, MAGIC

parser = argparse.ArgumentParser()
parser.add_argument("--port", type=int, default=17655)
parser.add_argument("--starter", action="store_true", help="Use the generated SL rig and test T-shirt")
parser.add_argument("--pbr", action="store_true", help="Exercise live image and PBR edits plus automatic weights")
parser.add_argument("--save-blend", type=Path, help="Save a self-contained PBR test scene")
args = parser.parse_args(sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else [])
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete(use_global=False)
if args.starter:
    rig, mesh = starter.create(bpy.context)
    print("AURORA_STARTER_RIG_BONES={} VERTICES={} QUADS={}".format(
        len(rig.data.bones), len(mesh.data.vertices), len(mesh.data.polygons)), flush=True)
else:
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
if args.pbr:
    weights.rebind(bpy.context, [mesh], rig)
    material = mesh.data.materials[0]
    material.use_nodes = True
    nodes, links = material.node_tree.nodes, material.node_tree.links
    shader = next(node for node in nodes if node.type == "BSDF_PRINCIPLED")
    def image_node(name, color, data=False):
        image = bpy.data.images.new(name, width=16, height=16, alpha=True)
        image.colorspace_settings.name = "Non-Color" if data else "sRGB"
        image.pixels.foreach_set(color * 256)
        image.update()
        node = nodes.new("ShaderNodeTexImage")
        node.image = image
        return node
    painted = image_node("Live color paint", [0.1,0.7,0.3,1])
    normal_image = image_node("Live normal", [0.5,0.5,1,1], True)
    orm_image = image_node("Live ORM", [1,0.6,0.2,1], True)
    normal = nodes.new("ShaderNodeNormalMap")
    separate = nodes.new("ShaderNodeSeparateRGB" if bpy.app.version < (3, 3, 0) else "ShaderNodeSeparateColor")
    links.new(painted.outputs["Color"], shader.inputs["Base Color"])
    links.new(painted.outputs["Color"], shader.inputs["Emission" if bpy.app.version < (4, 0, 0) else "Emission Color"])
    shader.inputs["Emission Strength"].default_value = 0.02
    links.new(normal_image.outputs["Color"], normal.inputs["Color"])
    links.new(normal.outputs["Normal"], shader.inputs["Normal"])
    links.new(orm_image.outputs["Color"], separate.inputs[0])
    links.new(separate.outputs[1], shader.inputs["Roughness"])
    links.new(separate.outputs[2], shader.inputs["Metallic"])
    materials.invalidate()
    if args.save_blend:
        for texture in (painted, normal_image, orm_image):
            texture.image.pack()
        from mathutils import Vector
        for screen in bpy.data.screens:
            for area in screen.areas:
                if area.type == "VIEW_3D":
                    space = area.spaces.active
                    space.show_region_ui = True
                    space.shading.type = "MATERIAL"
                    space.region_3d.view_rotation = Vector((4,-1,1)).to_track_quat("Z", "Y")
                    space.region_3d.view_location = (0,0,1)
                    space.region_3d.view_distance = 3.3
                    space.region_3d.view_perspective = "ORTHO"
        bpy.ops.wm.save_as_mainfile(filepath=str(args.save_blend.resolve()))

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
    if args.pbr:
        for revision in range(6, 13):
            shader.inputs["Alpha"].default_value = 0.6 if revision in {8,9} else 1.0
            pixels = []
            for y in range(16):
                for x in range(16):
                    pixels.extend([0.05,0.8,0.25,1] if ((x//4 + y//4 + revision) % 2) else [0.08,0.12,0.5,1])
            painted.image.pixels.foreach_set(pixels)
            painted.image.update()
            orm_image.image.pixels.foreach_set([1, 0.25 + (revision % 2) * 0.45, 0.15, 1] * 256)
            orm_image.image.update()
            materials.invalidate()
            value = exporter.snapshot([mesh], supported, revision, rest_axes=hello.get("rest_axes"))
            assert len(value["images"]) == 3, "base/emission share one image; normal and ORM are independent"
            assert all(index is not None for index in value["meshes"][0]["faces"][0]["material"]["textures"])
            send(value)
    reader.close()
print("AURORA_BLENDER_BRIDGE_SMOKE_OK", flush=True)
