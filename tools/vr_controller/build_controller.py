"""Aurora's VR controller model: an original hand controller in the
current style (a smooth shell, a dark teardrop face plate, a long curved
handle, a trigger paddle, a thumbstick and two face buttons; no tracking
ring), written as `crates/aurora-render/src/vr_controller.bin`.

Run inside Blender (4.2 or later), from its Python console or
`blender --background --python tools/vr_controller/build_controller.py`.
It builds the parts in a scene of their own, "Aurora VR controller", and
writes the mesh next to the renderer's source; nothing else in the open
file is touched, and the file is not saved. The shell and the face plate
are metaballs (one smooth surface each), reduced to a few thousand
triangles; the stick and the buttons are placed on the plate by ray casts.

The model is the right hand's; the renderer mirrors it for the left. Axes
are OpenXR's aim pose: +X right, +Y up, -Z where the controller points,
the origin at the aim point, in front of the head. In Blender the model is
built with +X right, +Z up and +Y forward, then converted on export.

The file, little-endian: b"AVRC", u32 version (2), u32 vertex count, the
thumbstick's pivot and the trigger's pivot (3 f32 each, aim-pose axes),
then per vertex of a triangle list: position (3 f32), normal (3 f32),
part (u32). Parts (the renderer colours and moves them):
    0 shell, 1 face plate, 2 trigger, 3 grip, 4 thumbstick,
    5 primary button (A/X), 6 secondary button (B/Y),
    7 menu button (drawn on the left hand only).
"""

import math
import os
import struct

import bmesh
import bpy
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

SHELL, FACE, TRIGGER, GRIP, STICK, PRIMARY, SECONDARY, MENU = range(8)

# Blender axes (x right, y forward, z up), metres; the head's centre
HEAD = Vector((0.0, -0.045, 0.0))
SCENE = "Aurora VR controller"

OUT = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "crates", "aurora-render", "src", "vr_controller.bin"
)


def scene():
    s = bpy.data.scenes.get(SCENE) or bpy.data.scenes.new(SCENE)
    for obj in list(s.collection.objects):
        data = obj.data
        bpy.data.objects.remove(obj, do_unlink=True)
        if data is not None and data.users == 0:
            if isinstance(data, bpy.types.MetaBall):
                bpy.data.metaballs.remove(data)
            elif isinstance(data, bpy.types.Mesh):
                bpy.data.meshes.remove(data)
    return s


def evaluated_mesh(s, obj):
    layer = s.view_layers[0]
    layer.update()
    return bpy.data.meshes.new_from_object(obj.evaluated_get(layer.depsgraph))


def meta_part(s, name, elements, triangles):
    """One smooth surface from metaball elements (centre, radius, half-size
    ratios or None for a ball), reduced to about `triangles`. The surface
    lies at about 0.65 of an element's radius. Built at ten times the size:
    Blender's finest metaball resolution is too coarse for a hand-sized
    thing."""
    big = 10.0
    mb = bpy.data.metaballs.new(name)
    mb.resolution = mb.render_resolution = 0.008
    mb.threshold = 0.6
    for co, radius, size in elements:
        e = mb.elements.new(type="ELLIPSOID" if size else "BALL")
        e.co = co * big
        e.radius = radius * big
        e.stiffness = 2.0
        if size:
            e.size_x, e.size_y, e.size_z = size
    meta = bpy.data.objects.new(name, mb)
    s.collection.objects.link(meta)
    mesh = evaluated_mesh(s, meta)
    mesh.transform(Matrix.Scale(1.0 / big, 4))
    bpy.data.objects.remove(meta, do_unlink=True)
    bpy.data.metaballs.remove(mb)
    obj = bpy.data.objects.new(name + "_mesh", mesh)
    s.collection.objects.link(obj)
    faces = max(len(mesh.polygons), 1)
    if faces > triangles:
        mod = obj.modifiers.new("reduce", "DECIMATE")
        mod.ratio = triangles / faces
        reduced = evaluated_mesh(s, obj)
        obj.modifiers.clear()
        obj.data = reduced
        bpy.data.meshes.remove(mesh)
    for poly in obj.data.polygons:
        poly.use_smooth = True
    return obj


def mesh_part(s, name, build):
    bm = bmesh.new()
    build(bm)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    for poly in mesh.polygons:
        poly.use_smooth = True
    obj = bpy.data.objects.new(name, mesh)
    s.collection.objects.link(obj)
    return obj


def dome(bm, center, normal, radius, height, segments=28, rings=8):
    """A flattened half-sphere standing on the plate along `normal`."""
    geom = bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=rings * 2, radius=1.0)
    for v in geom["verts"]:
        if v.co.z < 0:
            v.co.z *= 0.15
    rot = Vector((0, 0, 1)).rotation_difference(normal).to_matrix().to_4x4()
    m = Matrix.Translation(center) @ rot @ Matrix.Diagonal((radius, radius, height, 1.0))
    bmesh.ops.transform(bm, verts=geom["verts"], matrix=m)


def post_and_cap(bm, base, normal, post_height=0.0040):
    """The thumbstick: a short post and a wide cap dished on top, its edge
    rounded."""
    rot = Vector((0, 0, 1)).rotation_difference(normal).to_matrix().to_4x4()
    post = bmesh.ops.create_cone(bm, cap_ends=True, segments=16, radius1=0.0032, radius2=0.0028, depth=post_height)
    bmesh.ops.transform(
        bm, verts=post["verts"], matrix=Matrix.Translation(base) @ rot @ Matrix.Translation((0, 0, post_height / 2))
    )
    cap = bmesh.ops.create_uvsphere(bm, u_segments=36, v_segments=14, radius=1.0)
    for v in cap["verts"]:
        # a lens: flat-ish below, a shallow dish on top
        r = math.hypot(v.co.x, v.co.y)
        v.co.z = (v.co.z * 0.32) - (0.18 * (1.0 - r * r) if v.co.z > 0 else 0.0)
    m = (
        Matrix.Translation(base)
        @ rot
        @ Matrix.Translation((0, 0, post_height + 0.0018))
        @ Matrix.Diagonal((0.0088, 0.0088, 0.0088, 1.0))
    )
    bmesh.ops.transform(bm, verts=cap["verts"], matrix=m)


def handle_path(t):
    """The handle's spine: down and back from under the head, curving a
    little forward at the end, like a held banana."""
    return HEAD + Vector((0.0, -0.012 - 0.050 * t + 0.016 * t * t, -0.012 - 0.088 * t))


def surface_on(obj, x, y):
    """Where a ray straight down meets `obj` at (x, y): point and normal."""
    mesh = obj.data
    tree = BVHTree.FromPolygons([v.co for v in mesh.vertices], [p.vertices for p in mesh.polygons])
    point, normal, _, _ = tree.ray_cast(Vector((x, y, 0.2)), Vector((0, 0, -1)))
    if point is None:
        raise RuntimeError(f"nothing under ({x}, {y}) on {obj.name}")
    return point, normal if normal.z > 0 else -normal


def build(s):
    parts = []
    # the shell: the head under the plate and the handle, one surface
    # (the head flares up into the plate: a wide flat mass just under it,
    # and a narrower one where the handle leaves it)
    shell = [
        (HEAD + Vector((0.0, 0.004, 0.004)), 0.064, (0.86, 0.80, 0.30)),
        (HEAD + Vector((0.0, -0.010, -0.008)), 0.048, (0.75, 0.70, 0.55)),
    ]
    for i in range(9):
        t = i / 8
        radius = 0.033 - 0.005 * math.sin(t * math.pi * 0.9) + 0.002 * t
        shell.append((handle_path(t), radius, None))
    # (the targets count Blender's faces, quads mostly: about half the
    # triangles)
    parts.append((SHELL, meta_part(s, "shell", shell, 2600)))
    # the face plate: a rounded teardrop, wide at the front, overhanging
    # the shell a little, tilted down toward the front
    tilt = Matrix.Rotation(math.radians(-8), 3, "X")
    face = [
        (HEAD + tilt @ Vector((0.0, 0.006, 0.012)), 0.052, (0.92, 0.85, 0.16)),
        (HEAD + tilt @ Vector((0.002, -0.016, 0.011)), 0.040, (0.80, 0.70, 0.18)),
    ]
    plate = meta_part(s, "face_plate", face, 1100)
    parts.append((FACE, plate))
    # the trigger paddle, under the front, and the grip panel on the inner
    # side of the handle where the middle finger rests
    trigger_at = HEAD + Vector((0.0, 0.020, -0.020))
    trigger = [
        (trigger_at, 0.024, (0.45, 0.40, 0.75)),
        (trigger_at + Vector((0.0, -0.005, -0.012)), 0.020, (0.42, 0.38, 0.60)),
    ]
    parts.append((TRIGGER, meta_part(s, "trigger", trigger, 400)))
    grip_at = handle_path(0.35) + Vector((-0.019, 0.006, 0.0))
    parts.append((GRIP, meta_part(s, "grip", [(grip_at, 0.030, (0.25, 0.45, 0.95))], 300)))
    # on the plate: the stick (inner side), A/X and B/Y (outer side), menu
    stick_at, stick_n = surface_on(plate, -0.008, HEAD.y + 0.006)
    parts.append((STICK, mesh_part(s, "thumbstick", lambda bm: post_and_cap(bm, stick_at, stick_n))))
    parts.append(
        (FACE, mesh_part(s, "stick_well", lambda bm: dome(bm, stick_at - stick_n * 0.0006, stick_n, 0.0115, 0.0012, 32, 4)))
    )
    for part, x, y, radius in [
        (PRIMARY, 0.015, HEAD.y + 0.012, 0.0055),
        (SECONDARY, 0.013, HEAD.y - 0.006, 0.0055),
        (MENU, 0.004, HEAD.y - 0.021, 0.0034),
    ]:
        at, n = surface_on(plate, x, y)
        parts.append((part, mesh_part(s, f"button_{part}", lambda bm, at=at, n=n, r=radius: dome(bm, at, n, r, 0.0022))))
    trigger_pivot = trigger_at + Vector((0.0, -0.010, 0.010))
    return parts, stick_at, trigger_pivot


def aim(v):
    """Blender (x, y forward, z up) -> aim pose (x, y up, -z forward)."""
    return (v.x, v.z, -v.y)


def main():
    s = scene()
    parts, stick_pivot, trigger_pivot = build(s)
    data = bytearray()
    count = 0
    for part, obj in parts:
        mesh = obj.data
        mesh.calc_loop_triangles()
        normals = mesh.corner_normals
        for tri in mesh.loop_triangles:
            for loop in tri.loops:
                v = mesh.vertices[mesh.loops[loop].vertex_index].co
                n = normals[loop].vector
                data += struct.pack("<6fI", *aim(v), *aim(n), part)
                count += 1
    header = b"AVRC" + struct.pack("<II", 2, count) + struct.pack("<6f", *aim(stick_pivot), *aim(trigger_pivot))
    with open(os.path.normpath(OUT), "wb") as f:
        f.write(header + data)
    print(f"wrote {count} vertices ({count // 3} triangles) to {os.path.normpath(OUT)}")
    for part, obj in parts:
        print(f"  part {part}: {obj.name}, {len(obj.data.polygons)} faces")


main()
