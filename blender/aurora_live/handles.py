"""Selectable IK widgets: solid tubular strokes, native bone colors, no scene meshes.

Custom-shape transforms and color compatibility follow Blender's bone display
manual and 4.0 Bone Collections & Colors upgrade notes. Shapes live in local
bone coordinates and never change the IK target/pole transform or rest pose.
"""
import math
import bpy
from mathutils import Matrix, Vector

COLORS = {
    "L": ((0.12, 0.65, 0.95), (0.4, 0.85, 1), (0.72, 0.97, 1)),
    "R": ((1, 0.4, 0.1), (1, 0.66, 0.28), (1, 0.9, 0.64)),
}


class _Mesh:
    def __init__(self):
        self.vertices, self.faces = [], []

    def stroke(self, points, radius=0.055, closed=False):
        points = [Vector(p) for p in points]
        start, sides = len(self.vertices), 8
        for index, point in enumerate(points):
            before = points[(index - 1) % len(points)] if closed or index else point
            after = points[(index + 1) % len(points)] if closed or index + 1 < len(points) else point
            tangent = (after - before).normalized()
            ref = Vector((0, 0, 1)) if abs(tangent.z) < 0.9 else Vector((1, 0, 0))
            x = tangent.cross(ref).normalized()
            y = tangent.cross(x).normalized()
            for side in range(sides):
                angle = side * math.tau / sides
                self.vertices.append(point + radius * (math.cos(angle) * x + math.sin(angle) * y))
        for index in range(len(points) if closed else len(points) - 1):
            a = start + index * sides
            b = start + ((index + 1) % len(points)) * sides
            for side in range(sides):
                next_side = (side + 1) % sides
                self.faces.append((a + side, a + next_side, b + next_side, b + side))
        if not closed:
            self.faces += [tuple(start + i for i in reversed(range(sides))),
                           tuple(start + (len(points) - 1) * sides + i for i in range(sides))]

    def ring(self, plane, radius=1, offset=0):
        points = []
        for index in range(32):
            angle = index * math.tau / 32
            a, b = radius * math.cos(angle), radius * math.sin(angle)
            points.append((a, b, offset) if plane == "XY" else (a, offset, b) if plane == "XZ" else (offset, a, b))
        self.stroke(points, closed=True)

    def letter(self, side, y=0.3, z=0.12):
        paths = [[(-0.2, 0.35), (-0.2, -0.25), (0.2, -0.25)]] if side == "L" else [
            [(-0.2, -0.25), (-0.2, 0.35), (0.15, 0.35), (0.23, 0.2), (0.15, 0.05), (-0.2, 0.05)],
            [(0, 0.05), (0.25, -0.25)],
        ]
        for points in paths:
            self.stroke([(x, t + y, z) for x, t in points], radius=0.045)


def _geometry(style, side):
    mesh = _Mesh()
    flip = 1 if side == "L" else -1
    if style == "HAND":
        # A wrist cuff remains visible edge-on; the palm/thumb and arrow
        # distinguish the hand and indicate its forward direction.
        for y in (-0.2, 0.12):
            points = [(0.7 * math.cos(i * math.tau / 32), y, 0.48 * math.sin(i * math.tau / 32)) for i in range(32)]
            mesh.stroke(points, closed=True)
        for x in (-0.7, 0.7):
            mesh.stroke([(x, -0.2, 0), (x, 0.12, 0)])
        palm = [(-0.55, 0), (0.55, 0), (0.62, 0.8), (0.45, 1.4), (-0.35, 1.4),
                (-0.55, 0.85), (-0.95, 0.6), (-0.95, 0.25), (-0.6, 0.35)]
        mesh.stroke([(flip * x, y, 0) for x, y in palm], closed=True)
        mesh.stroke([(0, 1.4, 0), (0, 1.8, 0)])
        mesh.stroke([(-0.2, 1.6, 0), (0, 1.8, 0), (0.2, 1.6, 0)])
        mesh.letter(side, y=0.65, z=0.12)
    elif style == "FOOT":
        sole = [(-0.4, -0.6), (0.4, -0.6), (0.55, -0.3), (0.57, 0.85),
                (0.35, 1.35), (-0.25, 1.42), (-0.6, 1.1), (-0.55, 0.2)]
        for z in (-0.45, -0.15):
            mesh.stroke([(flip * x * 0.65, y, z) for x, y in sole], closed=True)
        for index in (0, 1, 3, 5):
            x, y = sole[index]
            mesh.stroke([(flip * x * 0.65, y, -0.45), (flip * x * 0.65, y, -0.15)])
        mesh.ring("XY", radius=0.37, offset=0.15)
        mesh.letter(side, y=0.55, z=-0.08)
    elif style == "ELBOW":
        points = [(0.9, 0, 0), (0, 0.9, 0), (-0.9, 0, 0), (0, -0.9, 0)]
        mesh.stroke(points, closed=True)
        for tip in ((0, 0, 0.9), (0, 0, -0.9)):
            for point in points:
                mesh.stroke([point, tip])
        mesh.letter(side, z=1)
    else:
        for plane in ("XY", "XZ", "YZ"):
            mesh.ring(plane)
        mesh.letter(side, z=1.1)
    return mesh


def _shape(style, side, frame):
    signature = style + side + ":2:" + repr(tuple(round(value, 5) for row in frame for value in row))
    obj = next((obj for obj in bpy.data.objects if obj.get("aurora_ik_shape") == signature), None)
    if obj is None:
        geometry = _geometry(style, side)
        mesh = bpy.data.meshes.new("_AuroraIKShape." + style + "." + side)
        mesh.from_pydata([frame @ p for p in geometry.vertices], [], geometry.faces)
        mesh.update()
        for face in mesh.polygons:
            face.use_smooth = True
        obj = bpy.data.objects.new(mesh.name, mesh)
        obj["aurora_ik_shape"] = signature
        # Referenced by PoseBone.custom_shape, never linked to a scene.
    return obj


def apply(rig, replace=False):
    from . import rigging
    if rig.type != "ARMATURE" or rig.library or rig.data.library or rig.mode == "EDIT" or not rigging.ready(rig):
        return
    factor = getattr(rig, "aurora_live_ik_handle_size", 1)
    for key in rigging.LIMBS:
        names = rigging.chain(rig, key)
        root, middle, end = (rig.data.bones[name] for name in names)
        length = (middle.head_local - root.head_local).length
        side = key[-1]
        for pole in (False, True):
            bone = rig.pose.bones[rigging.control_name(key, pole)]
            if not bone.bone.get("aurora_control"):
                continue
            if bone.custom_shape and not bone.custom_shape.get("aurora_ik_shape") and not replace:
                continue
            hand = key.startswith("HAND")
            style = ("ELBOW" if hand else "KNEE") if pole else ("HAND" if hand else "FOOT")
            frame = Matrix.Identity(3)
            if style == "FOOT":
                # Keep the sole horizontal in rest space, even though SL's
                # ankle bone points down/forward. Derive -Y-forward rigs too.
                forward = end.tail_local - end.head_local
                foot_name = "mFoot" + ("Left" if side == "L" else "Right")
                foot = next((b for b in rig.data.bones if b.get("aurora_joint", b.name) == foot_name), None)
                if foot:
                    forward = foot.head_local - end.head_local
                forward.z = 0
                if forward.length < 1e-6:
                    forward = Vector((1, 0, 0))
                forward.normalize()
                up = Vector((0, 0, 1))
                across = up.cross(forward)
                frame = end.matrix_local.to_3x3().inverted() @ Matrix((across, forward, up)).transposed()
            bone.custom_shape = _shape(style, side, frame)
            size = length * ((0.22 if hand else 0.14) if pole else (0.42 if hand else 0.36)) * factor
            bone.use_custom_shape_bone_size = False
            if hasattr(bone, "custom_shape_scale_xyz"):
                bone.custom_shape_scale_xyz = (size,) * 3
            else:
                bone.custom_shape_scale = size
            bone.bone.show_wire = False
            if hasattr(bone, "custom_shape_wire_width"):
                bone.custom_shape_wire_width = 2
            if hasattr(bone, "color"):
                bone.color.palette = "CUSTOM"
                colors = bone.color.custom
            else:
                label = "Aurora IK " + ("Left" if side == "L" else "Right")
                group = rig.pose.bone_groups.get(label) or rig.pose.bone_groups.new(name=label)
                group.color_set = "CUSTOM"
                bone.bone_group = group
                colors = group.colors
            colors.normal, colors.select, colors.active = COLORS[side]
