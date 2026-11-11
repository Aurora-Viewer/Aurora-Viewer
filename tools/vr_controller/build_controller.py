"""Aurora's VR controller model: an original, ringless (Touch Plus style) hand controller
built from simple shapes, written as `crates/aurora-render/src/vr_controller.bin`.

Run inside Blender (4.2 or later), from its Python console or
`blender --background --python tools/vr_controller/build_controller.py`.
It builds the parts in a scene of their own, "Aurora VR controller", and
writes the mesh next to the renderer's source; nothing else in the open
file is touched, and the file is not saved.

The model is the right hand's; the renderer mirrors it for the left. Axes
are OpenXR's aim pose: +X right, +Y up, -Z where the controller points,
the origin at the aim point, in front of the head. In Blender the model is
built with +X right, +Z up and +Y forward, then converted on export.

The file: b"AVRC", u32 version (1), u32 vertex count, then per vertex of a
triangle list: position (3 f32), normal (3 f32), part (u32), little-endian.
Parts (the renderer colours and moves them):
    0 body, 1 (unused: no tracking ring), 2 trigger, 3 grip, 4 thumbstick, 5 primary button (A/X),
    6 secondary button (B/Y), 7 menu button (drawn on the left hand only).
The pivots the renderer turns the thumbstick and trigger about are
`STICK_PIVOT` and `TRIGGER_PIVOT` below, in aim-pose axes, and must match
`vr_pointer.rs`.
"""

import math
import os
import struct

import bmesh
import bpy
from mathutils import Matrix, Vector

BODY, RING, TRIGGER, GRIP, STICK, PRIMARY, SECONDARY, MENU = range(8)

# Blender axes (x right, y forward, z up), metres
HEAD = Vector((0.0, -0.045, 0.0))
HANDLE_AXIS = Vector((0.0, -0.55, -0.83)).normalized()
HANDLE_LENGTH = 0.10
HANDLE_RADIUS = 0.0165
STICK_BASE = Vector((-0.004, -0.050, 0.0165))
TRIGGER_AT = Vector((0.0, -0.020, -0.015))

# the same pivots in aim-pose axes (x, z, -y), for vr_pointer.rs
STICK_PIVOT = (STICK_BASE.x, STICK_BASE.z, -STICK_BASE.y)
TRIGGER_PIVOT = (TRIGGER_AT.x, TRIGGER_AT.z + 0.004, -(TRIGGER_AT.y - 0.006))

OUT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates", "aurora-render", "src", "vr_controller.bin"
)


def ellipsoid(bm, center, radii, segments=32, rings=16):
    geom = bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=rings, radius=1.0)
    verts = [v for v in geom["verts"]]
    bmesh.ops.transform(bm, verts=verts, matrix=Matrix.Translation(center) @ Matrix.Diagonal((*radii, 1.0)))
    return verts


def cylinder(bm, start, end, radius, segments=24):
    axis = end - start
    geom = bmesh.ops.create_cone(
        bm, cap_ends=True, cap_tris=False, segments=segments, radius1=radius, radius2=radius, depth=axis.length
    )
    rot = Vector((0.0, 0.0, 1.0)).rotation_difference(axis.normalized()).to_matrix().to_4x4()
    bmesh.ops.transform(bm, verts=geom["verts"], matrix=Matrix.Translation((start + end) * 0.5) @ rot)
    return geom["verts"]


def part_object(name, build):
    """A mesh object for one part, built by `build(bm)`, smooth shaded."""
    bm = bmesh.new()
    build(bm)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    for poly in mesh.polygons:
        poly.use_smooth = True
    obj = bpy.data.objects.new(name, mesh)
    return obj


def box_part(name, center, half, rotation=Matrix.Identity(3)):
    def build(bm):
        geom = bmesh.ops.create_cube(bm, size=2.0)
        bmesh.ops.bevel(bm, geom=list(bm.edges), offset=0.3, segments=3, affect="EDGES")
        m = Matrix.Translation(center) @ rotation.to_4x4() @ Matrix.Diagonal((*half, 1.0))
        bmesh.ops.transform(bm, verts=list(bm.verts), matrix=m)

    return part_object(name, build)


def build_parts():
    parts = []
    # the head: a flattened egg the ring wraps
    parts.append((BODY, part_object("head", lambda bm: ellipsoid(bm, HEAD, (0.026, 0.034, 0.0175)))))
    # the handle, angled down and back, with a rounded end
    start = HEAD + HANDLE_AXIS * 0.012
    end = HEAD + HANDLE_AXIS * HANDLE_LENGTH

    def handle(bm):
        cylinder(bm, start, end, HANDLE_RADIUS)
        ellipsoid(bm, end, (HANDLE_RADIUS, HANDLE_RADIUS, HANDLE_RADIUS), 24, 12)

    parts.append((BODY, part_object("handle", handle)))
    # thumbstick: a base, a shaft and a cap on top of the head
    def stick(bm):
        cylinder(bm, STICK_BASE, STICK_BASE + Vector((0.0, 0.0, 0.009)), 0.0028, 12)
        cylinder(bm, STICK_BASE + Vector((0.0, 0.0, 0.009)), STICK_BASE + Vector((0.0, 0.0, 0.0125)), 0.0085, 24)

    parts.append((STICK, part_object("thumbstick", stick)))
    parts.append((BODY, part_object("stick_well", lambda bm: cylinder(bm, STICK_BASE - Vector((0, 0, 0.001)), STICK_BASE + Vector((0, 0, 0.0015)), 0.0105, 24))))
    # face buttons, on the outer side of the stick (A/X lower, B/Y higher)
    top = 0.0158
    parts.append((PRIMARY, part_object("button_primary", lambda bm: cylinder(bm, Vector((0.012, -0.040, top)), Vector((0.012, -0.040, top + 0.0035)), 0.0048, 20))))
    parts.append((SECONDARY, part_object("button_secondary", lambda bm: cylinder(bm, Vector((0.0095, -0.058, top - 0.001)), Vector((0.0095, -0.058, top + 0.0025)), 0.0045, 20))))
    parts.append((MENU, part_object("button_menu", lambda bm: cylinder(bm, Vector((0.016, -0.064, top - 0.004)), Vector((0.016, -0.064, top - 0.0015)), 0.0028, 16))))
    # trigger: under the front of the head, tilted toward the hand
    parts.append((TRIGGER, box_part("trigger", TRIGGER_AT, (0.0085, 0.010, 0.0045), Matrix.Rotation(math.radians(-35), 3, "X"))))
    # grip button: on the front of the handle, where the middle finger rests
    front = Vector((0.0, 0.83, -0.55)).normalized()
    grip_at = HEAD + HANDLE_AXIS * 0.055 + front * (HANDLE_RADIUS - 0.002)
    grip_rot = Vector((0.0, 0.0, 1.0)).rotation_difference(HANDLE_AXIS).to_matrix()
    parts.append((GRIP, box_part("grip", grip_at, (0.0085, 0.006, 0.020), grip_rot)))
    return parts


def main():
    scene = bpy.data.scenes.get("Aurora VR controller") or bpy.data.scenes.new("Aurora VR controller")
    for obj in list(scene.collection.objects):
        bpy.data.objects.remove(obj, do_unlink=True)
    parts = build_parts()
    data = bytearray()
    count = 0
    for part, obj in parts:
        scene.collection.objects.link(obj)
        mesh = obj.data
        mesh.calc_loop_triangles()
        normals = mesh.corner_normals
        for tri in mesh.loop_triangles:
            for loop in tri.loops:
                v = mesh.vertices[mesh.loops[loop].vertex_index].co
                n = normals[loop].vector
                # Blender (x, y forward, z up) -> aim pose (x, y up, -z forward)
                data += struct.pack("<6fI", v.x, v.z, -v.y, n.x, n.z, -n.y, part)
                count += 1
    with open(os.path.normpath(OUT), "wb") as f:
        f.write(b"AVRC" + struct.pack("<II", 1, count) + data)
    print(f"wrote {count} vertices ({count // 3} triangles) to {os.path.normpath(OUT)}")
    print(f"STICK_PIVOT = {STICK_PIVOT}, TRIGGER_PIVOT = {TRIGGER_PIVOT}")


main()
