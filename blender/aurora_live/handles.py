"""Selectable IK widgets: clean continuous outlines, native colors, no scene meshes.

Custom-shape transforms and color compatibility follow Blender's bone display
manual and 4.0 Bone Collections & Colors upgrade notes. Shapes live in local
bone coordinates and never change the IK target/pole transform or rest pose.
"""
import math
import bpy
from mathutils import Matrix, Vector

COLORS = {
    "L": ((0.18, 0.68, 0.78), (0.38, 0.86, 0.92), (0.78, 1, 1)),
    "R": ((0.96, 0.43, 0.30), (1, 0.67, 0.50), (1, 0.9, 0.76)),
}


class _Mesh:
    def __init__(self):
        self.vertices, self.edges = [], []

    def stroke(self, points, closed=False):
        start = len(self.vertices)
        self.vertices.extend(Vector(p) for p in points)
        self.edges.extend((start + i, start + i + 1) for i in range(len(points) - 1))
        if closed:
            self.edges.append((len(self.vertices) - 1, start))

    def ring(self, plane, radius=1, offset=0):
        points = []
        for index in range(64):
            angle = index * math.tau / 64
            a, b = radius * math.cos(angle), radius * math.sin(angle)
            points.append((a, b, offset) if plane == "XY" else (a, offset, b) if plane == "XZ" else (offset, a, b))
        self.stroke(points, closed=True)

    def rounded(self, width, height, radius, plane, center=(0, 0, 0), angle=0):
        points = []
        for x, y, first in ((width/2-radius, height/2-radius, 0), (-width/2+radius, height/2-radius, 90),
                            (-width/2+radius, -height/2+radius, 180), (width/2-radius, -height/2+radius, 270)):
            for step in range(13):
                t = math.radians(first + step * 90 / 12)
                a, b = x + radius * math.cos(t), y + radius * math.sin(t)
                a, b = a * math.cos(angle) - b * math.sin(angle), a * math.sin(angle) + b * math.cos(angle)
                point = Vector((a, b, 0) if plane == "XY" else (a, 0, b) if plane == "XZ" else (0, a, b))
                points.append(point + Vector(center))
        self.stroke(points, closed=True)


def _geometry(style, side):
    mesh = _Mesh()
    if style == "HAND":
        # Wrist local Y points toward the fingers; XY follows the palm.
        mesh.rounded(1.3, 1.8, 0.28, "XY", center=(0, 0.3, 0))
        mesh.stroke([(-0.18, 1.02, 0), (0.18, 1.02, 0)])
    elif style == "FOOT":
        mesh.rounded(0.82, 1.9, 0.28, "XY", center=(0, 0.35, -0.32))
        mesh.stroke([(0.32 * math.cos(i * math.pi / 24), -0.35, -0.32 + 0.44 * math.sin(i * math.pi / 24))
                     for i in range(25)])
    elif style == "ELBOW":
        mesh.rounded(1.3, 1.3, 0.18, "YZ", angle=math.pi/4)
        mesh.stroke([(0, -0.12, 0), (0, 0.12, 0)])
        mesh.stroke([(0, 0, -0.12), (0, 0, 0.12)])
    else:
        mesh.ring("XZ", radius=0.9)
        mesh.stroke([(-0.12, 0, 0), (0.12, 0, 0)])
        mesh.stroke([(0, 0, -0.12), (0, 0, 0.12)])
    return mesh


def _shape(style, side, frame):
    revision = "4" if style == "HAND" else "3"
    signature = style + side + ":" + revision + ":" + repr(tuple(round(value, 5) for row in frame for value in row))
    obj = next((obj for obj in bpy.data.objects if obj.get("aurora_ik_shape") == signature), None)
    if obj is None:
        geometry = _geometry(style, side)
        mesh = bpy.data.meshes.new("_AuroraIKShape." + style + "." + side)
        mesh.from_pydata([frame @ p for p in geometry.vertices], geometry.edges, [])
        mesh.update()
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
            bone.bone.show_wire = True
            if hasattr(bone, "custom_shape_wire_width"):
                bone.custom_shape_wire_width = 3
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
