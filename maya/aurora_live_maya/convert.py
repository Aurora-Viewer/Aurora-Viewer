"""Pure conversion rules of the Maya plug-in: no Maya import, unit-tested alone.

Maya works in centimetres with Y up; LiveMesh snapshots are in metres in Second
Life's Z-up frame (docs/BLENDER_LIVE.md, "Wire protocol v1"). The Y-up turn is
the one the Legacy Viewer applies to a Y_UP Collada file (LLDAELoader::
OpenFile, indra/llprimitive/lldaeloader.cpp: a 90 degree rotation about X), so
a mesh previewed live stands as it would after a Collada upload. A rig is also
turned to face +X (facing()), which an upload would not do: a rig that does not
face +X previews right and still needs turning before its upload. The weight
and facing rules follow the Blender plug-in (blender/aurora_live/exporter.py).

Matrices here are lists of four rows, applied to column vectors (M @ v). A Maya
MMatrix is the transpose (row vectors, translation in its last row).
"""

import math
import struct

MAX_CORNERS = 600_000
MAX_SURFACES = 32
MAX_JOINTS = 110
VERTEX = struct.Struct("<8f4B4f")
CM = 0.01


# ---------------------------------------------------------------- matrices

def identity():
    return [[1.0 if row == column else 0.0 for column in range(4)] for row in range(4)]


def multiply(a, b):
    return [[sum(a[row][k] * b[k][column] for k in range(4)) for column in range(4)] for row in range(4)]


def chain(*matrices):
    result = identity()
    for matrix in matrices:
        result = multiply(result, matrix)
    return result


def from_maya(values):
    """A Maya matrix (16 floats, row-major, row vectors) in column-vector form."""
    values = [float(value) for value in values]
    if len(values) != 16 or not all(math.isfinite(value) for value in values):
        raise ValueError("Invalid matrix")
    return [[values[column * 4 + row] for column in range(4)] for row in range(4)]


def column_major(matrix):
    return [float(matrix[row][column]) for column in range(4) for row in range(4)]


def from_column_major(values):
    return [[float(values[column * 4 + row]) for column in range(4)] for row in range(4)]


def translation(vector):
    result = identity()
    for axis in range(3):
        result[axis][3] = float(vector[axis])
    return result


def translation_of(matrix):
    return [matrix[0][3], matrix[1][3], matrix[2][3]]


def linear(matrix):
    """The 3x3 part as a 4x4 matrix, translation removed."""
    result = identity()
    for row in range(3):
        for column in range(3):
            result[row][column] = matrix[row][column]
    return result


def scale(factor):
    result = identity()
    for axis in range(3):
        result[axis][axis] = float(factor)
    return result


def rotation_z(angle):
    c, s = math.cos(angle), math.sin(angle)
    result = identity()
    result[0][0], result[0][1], result[1][0], result[1][1] = c, -s, s, c
    return result


def inverse(matrix):
    """Gauss-Jordan inverse of a 4x4 matrix; refuses a singular one."""
    work = [list(map(float, row)) + [1.0 if row_index == column else 0.0 for column in range(4)]
            for row_index, row in enumerate(matrix)]
    for column in range(4):
        pivot = max(range(column, 4), key=lambda row: abs(work[row][column]))
        if abs(work[pivot][column]) < 1e-12:
            raise ValueError("Singular matrix")
        work[column], work[pivot] = work[pivot], work[column]
        factor = work[column][column]
        work[column] = [value / factor for value in work[column]]
        for row in range(4):
            if row != column and work[row][column]:
                ratio = work[row][column]
                work[row] = [value - ratio * pivot_value for value, pivot_value in zip(work[row], work[column])]
    return [row[4:] for row in work]


def determinant3(matrix):
    m = matrix
    return (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]))


def point(matrix, vector):
    x, y, z = vector
    return [matrix[row][0] * x + matrix[row][1] * y + matrix[row][2] * z + matrix[row][3] for row in range(3)]


def direction(matrix, vector):
    x, y, z = vector
    return [matrix[row][0] * x + matrix[row][1] * y + matrix[row][2] * z for row in range(3)]


def normal_matrix(matrix):
    """Inverse transpose of the linear part, for normals."""
    inverted = inverse(linear(matrix))
    return [[inverted[column][row] for column in range(4)] for row in range(4)]


# Maya world (Y up) to Second Life (Z up): (x, y, z) -> (x, -z, y), the Legacy
# Viewer's rotation of a Y_UP Collada file. A Z-up Maya scene needs no turn.
Y_UP = [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, -1.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]]


def base_frame(up_axis="y", unit=CM):
    """Maya world to the SL frame: `unit` metres per Maya unit, then the turn
    of a Y-up scene."""
    scaled = scale(unit)
    return multiply(Y_UP, scaled) if str(up_axis).lower() == "y" else scaled


TO_SL = base_frame("y", CM)

# Metres per Maya unit a scene can be built in: real centimetres (Maya's own
# unit), or one unit per metre as many Second Life kits are, whatever unit the
# Maya preferences name.
UNITS = {"cm": CM, "m": 1.0}
SCALES = (1.0, 0.1, 0.01, 0.001)
# Second Life's mPelvis stands about a metre above the feet (avatar_skeleton.xml).
PELVIS_HEIGHT = 1.0


def rig_unit(heads, up_axis="y"):
    """Metres per Maya unit of a Second Life rig, from its pelvis height above
    its lowest joint, or None when the rig has no mPelvis.

    `heads` maps SL joint names to world positions in Maya units. The scale
    that brings the pelvis nearest a metre above the feet wins, among the
    powers of ten a scene is built in.
    """
    pelvis = heads.get("mPelvis")
    if pelvis is None:
        return None
    up = 1 if str(up_axis).lower() == "y" else 2
    lowest = min(head[up] for head in heads.values())
    height = pelvis[up] - lowest
    if not math.isfinite(height) or height <= 1e-9:
        return None
    return min(SCALES, key=lambda unit: abs(math.log(height * unit / PELVIS_HEIGHT)))


# ------------------------------------------------------------------ joints

def joint_name(name, override=None):
    """A Maya joint's Second Life name: its `auroraJoint` string when set,
    otherwise its short name without namespace (`rig:mPelvis` -> `mPelvis`)."""
    if override:
        return str(override).strip()
    return str(name).rsplit("|", 1)[-1].rsplit(":", 1)[-1]


def facing(heads):
    """The turn about Z that brings a rig's left side to SL +Y.

    `heads` maps SL joint names to positions already in the Z-up frame. Like
    the Blender plug-in's coordinate_basis, symmetric pairs say where the rig's
    left is without trusting joint orientation; SL avatars face +X, left +Y.
    """
    for stem in ("mShoulder", "mCollar", "mHip", "mEye", "mWrist"):
        left, right = heads.get(stem + "Left"), heads.get(stem + "Right")
        if left is None or right is None:
            continue
        dx, dy = left[0] - right[0], left[1] - right[1]
        if dx * dx + dy * dy < 1e-12:
            continue
        angle = math.remainder(math.pi / 2 - math.atan2(dy, dx), math.tau)
        return identity() if abs(angle) < 1e-6 else rotation_z(angle)
    return identity()


def inverse_binds(names, bind_pre, frame, axes_mode=False, rest_axes=None, shaped=None):
    """Inverse bind matrices in SL's frame, column-major.

    `bind_pre` maps each joint name to its skinCluster bindPreMatrix (inverse
    of the joint's world matrix at bind time) in column-vector form, Maya
    units. `frame` takes Maya world to the SL frame. By default the matrix is
    the skin's own, as a Collada export from Maya writes it. With `axes_mode`
    the joint's orientation is replaced by the viewer's neutral joint axes
    (`rest_axes`, from its hello), keeping only its bind position: the Blender
    plug-in's rule for bones without bind data, for rigs whose joint
    orientations do not follow Second Life's.

    `shaped` maps a joint received from Aurora's shape transfer to that
    shape's (inverse bind, head), both in SL metres: its inverse bind cancels
    the shape, scale included, before Aurora applies the wearer's own, offset
    by how far the bind head sits from the shape's head (the Blender
    plug-in's aurora_shape_bind).
    """
    frame_inverse = inverse(frame)
    result = []
    for name in names:
        converted = chain(frame, bind_pre[name], frame_inverse)
        if shaped and name in shaped:
            bind, reference = shaped[name]
            head = translation_of(inverse(converted))
            matrix = from_column_major(bind)
            if abs(determinant3(matrix)) < 1e-8:
                raise ValueError("Invalid transferred shape bind: " + name)
            result.append(column_major(multiply(matrix, translation([r - h for r, h in zip(reference, head)]))))
            continue
        if axes_mode:
            head = translation_of(inverse(converted))
            values = (rest_axes or {}).get(name)
            axes = from_column_major(values) if values and len(values) == 16 else identity()
            converted = multiply(linear(axes), translation([-value for value in head]))
        result.append(column_major(converted))
    return result


def rank(influences):
    """Strongest four of (joint index, weight), normalized, like the Blender
    plug-in's influences(); ties go to the lower joint index."""
    values = []
    for joint, weight in influences:
        weight = float(weight)
        if not math.isfinite(weight):
            raise ValueError("Avatar joint weights must be finite")
        if weight > 0.0:
            values.append((joint, weight))
    values.sort(key=lambda pair: (-pair[1], pair[0]))
    values = values[:4]
    total = sum(weight for _, weight in values)
    if total <= 0.000001:
        return (0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)
    joints = [joint for joint, _ in values] + [0] * (4 - len(values))
    weights = [weight / total for _, weight in values] + [0.0] * (4 - len(values))
    return tuple(joints), tuple(weights)


class Nearest:
    """Nearest of a set of points, distances normalized by `extent` on each
    axis; ties go to the lower index. A uniform grid keeps it fast."""

    def __init__(self, points, indices, extent):
        self.points, self.extent = points, extent
        self.cells = max(1, int(round(len(indices) ** (1.0 / 3.0))))
        self.size = 1.0 / self.cells
        self.grid = {}
        for index in indices:
            p = self.normalized(points[index])
            self.grid.setdefault(self.cell(p), []).append((p, index))

    def normalized(self, point):
        return tuple(point[axis] / self.extent[axis] for axis in range(3))

    def cell(self, p):
        return tuple(int(math.floor(value / self.size)) for value in p)

    def find(self, point):
        p = self.normalized(point)
        cell = self.cell(p)
        best, best_distance, ring = None, math.inf, 0
        while best is None or (ring - 1) * self.size <= math.sqrt(best_distance):
            for dx in range(-ring, ring + 1):
                for dy in range(-ring, ring + 1):
                    for dz in range(-ring, ring + 1):
                        if max(abs(dx), abs(dy), abs(dz)) != ring:
                            continue
                        for q, candidate in self.grid.get((cell[0] + dx, cell[1] + dy, cell[2] + dz), ()):
                            distance = sum((a - b) ** 2 for a, b in zip(p, q))
                            if distance < best_distance or (distance == best_distance and candidate < best):
                                best, best_distance = candidate, distance
            ring += 1
            if best is None and ring > self.cells * 4 + 64:
                # Far outside the grid: compare with every point.
                return min((index for cell_points in self.grid.values() for _q, index in cell_points),
                           key=lambda index: (sum((a - b) ** 2 for a, b in zip(p, self.normalized(self.points[index]))), index))
        return best


def extent_of(points, indices):
    low = [min(points[index][axis] for index in indices) for axis in range(3)]
    high = [max(points[index][axis] for index in indices) for axis in range(3)]
    return [max(high[axis] - low[axis], 1e-6) for axis in range(3)]


def complete(joints, weights, positions, referenced):
    """Give each referenced vertex without weights those of the nearest weighted
    one, distances normalized by the mesh's extent on each axis: mesh import's
    rule, as in the Blender plug-in. All unweighted: the first joint, whole."""
    missing = [index for index in referenced if not any(weights[index])]
    if not missing:
        return
    weighted = [index for index in referenced if any(weights[index])]
    if not weighted:
        for index in missing:
            joints[index], weights[index] = (0, 0, 0, 0), (1.0, 0.0, 0.0, 0.0)
        return
    nearest = Nearest(positions, weighted, extent_of(positions, referenced))
    for index in missing:
        best = nearest.find(positions[index])
        joints[index], weights[index] = joints[best], weights[best]


def transfer(source_positions, source_joints, source_weights, source_referenced, positions):
    """Weights of each point from the nearest source vertex: how an automatic
    LOD, reduced from High at rest, keeps High's skin."""
    nearest = Nearest(source_positions, source_referenced, extent_of(source_positions, source_referenced))
    joints, weights = [], []
    for point in positions:
        best = nearest.find(point)
        joints.append(source_joints[best])
        weights.append(source_weights[best])
    return joints, weights


# ---------------------------------------------------------------- surfaces

def triangle_corners(counts, offsets, polygon_sizes):
    """Face-vertex index triples from MFnMesh.getTriangleOffsets.

    The offsets index the mesh's face-vertex list. Older documentation reads
    as if they were relative to their polygon: both are accepted, told apart
    by an offset no polygon could hold.
    """
    triangles, polygons = [], []
    starts, start = [], 0
    for size in polygon_sizes:
        starts.append(start)
        start += size
    # A polygon's triangles use all its corners: relative offsets start at 0,
    # global ones at the polygon's first face-vertex.
    relative = _offsets_are_relative(counts, offsets, starts)
    position = 0
    for polygon, count in enumerate(counts):
        base = starts[polygon] if relative else 0
        for _ in range(count):
            triangles.append((base + offsets[position], base + offsets[position + 1], base + offsets[position + 2]))
            polygons.append(polygon)
            position += 3
    return triangles, polygons


def _offsets_are_relative(counts, offsets, starts):
    position = 0
    for polygon, count in enumerate(counts):
        values = offsets[position:position + count * 3]
        position += count * 3
        if values and starts[polygon] > 0 and min(values) < starts[polygon]:
            return True
    return False


def surfaces(positions, normals, uvs, fv_vertex, fv_normal, fv_uv, triangles, triangle_slots, flip,
             joints=None, weights=None):
    """Per material slot, the packed vertex records and u32 indices the viewer
    reads (52-byte records: position, normal, UV, four joints, four weights).

    A corner is one vertex with one normal and one UV; equal corners of a slot
    share a record. `flip` reverses the winding of a mirrored transform.
    """
    result = {}
    zero_joints, zero_weights = (0, 0, 0, 0), (0.0, 0.0, 0.0, 0.0)
    pack = VERTEX.pack
    for triangle, slot in zip(triangles, triangle_slots):
        records, indices, lookup = result.setdefault(slot, (bytearray(), [], {}))
        for corner in (reversed(triangle) if flip else triangle):
            vertex, normal, uv = fv_vertex[corner], fv_normal[corner], fv_uv[corner]
            key = (vertex, normal, uv)
            index = lookup.get(key)
            if index is None:
                index = len(lookup)
                lookup[key] = index
                u, v = uvs[uv] if uv >= 0 else (0.0, 0.0)
                records += pack(*positions[vertex], *normals[normal], u, v,
                                *(joints[vertex] if joints is not None else zero_joints),
                                *(weights[vertex] if weights is not None else zero_weights))
            indices.append(index)
    packed = {}
    for slot, (records, indices, _lookup) in result.items():
        packed[slot] = (bytes(records), struct.pack("<{}I".format(len(indices)), *indices))
    return packed


def corner_count(packed):
    return sum(len(records) // VERTEX.size for records, _indices in packed.values())


# ---------------------------------------------------------------- linksets

def static_roots(parents):
    """Map each static source to its linkset root.

    `parents` maps a source key to the key of its nearest synchronized static
    ancestor in the Maya hierarchy (None for none), like the Blender plug-in's
    inherited parenting. A cycle cannot come from a DAG but is refused.
    """
    roots = {}
    for key in parents:
        seen, current = set(), key
        while parents.get(current) is not None:
            if current in seen:
                raise ValueError("Invalid LiveMesh grouping")
            seen.add(current)
            current = parents[current]
        roots[key] = current
    return roots


# --------------------------------------------------------------- materials

def uv_transform(repeat=(1.0, 1.0), offset=(0.0, 0.0)):
    """A place2dTexture's repeat and offset as the viewer's texture transform
    ([offset U, offset V, scale U, scale V, rotation]).

    Maya samples at uv * repeat + offset with V up; the viewer's transform is
    glTF's, with V down: v' = v * repeat + (1 - repeat - offset). The same
    form as the Blender plug-in's Mapping node with no rotation.
    """
    ru, rv = float(repeat[0]), float(repeat[1])
    ou, ov = float(offset[0]), float(offset[1])
    return [ou, 1.0 - ov - rv, ru, rv, 0.0]


def _srgb_to_linear(value):
    value /= 255.0
    return value / 12.92 if value <= 0.04045 else ((value + 0.055) / 1.055) ** 2.4


def _linear_to_srgb(value):
    value /= 255.0
    return value * 12.92 if value <= 0.0031308 else 1.055 * value ** (1 / 2.4) - 0.055


TO_LINEAR = bytes(int(round(min(1.0, max(0.0, _srgb_to_linear(value))) * 255)) for value in range(256))
TO_SRGB = bytes(int(round(min(1.0, max(0.0, _linear_to_srgb(value))) * 255)) for value in range(256))


def recode(rgba, table):
    """Apply a byte table to RGB, leaving alpha as it is."""
    converted = bytearray(rgba.translate(table))
    converted[3::4] = rgba[3::4]
    return bytes(converted)


def channel(rgba, index):
    """One channel of RGBA8 repeated as an opaque grey image."""
    value = rgba[index::4]
    result = bytearray(len(rgba))
    result[0::4] = value
    result[1::4] = value
    result[2::4] = value
    result[3::4] = b"\xff" * len(value)
    return bytes(result)


def opaque(rgba):
    result = bytearray(rgba)
    result[3::4] = b"\xff" * (len(rgba) // 4)
    return bytes(result)


def orm(size, roughness=None, metallic=None):
    """Pack roughness into G and metallic into B of an ORM image (occlusion
    and alpha 1), each from (RGBA8 bytes, channel index) of the same size."""
    width, height = size
    result = bytearray(b"\xff" * (width * height * 4))
    for source, target in ((roughness, 1), (metallic, 2)):
        if source is not None:
            pixels, index = source
            if len(pixels) != len(result):
                raise ValueError("Roughness and metalness images must have the same size")
            result[target::4] = pixels[index::4]
    return bytes(result)


def fit(width, height, limit=1024):
    """The size an image is scaled to before it is sent, like the Blender
    plug-in's 1024 pixel cap; never enlarged."""
    if width <= 0 or height <= 0:
        raise ValueError("Empty image")
    factor = min(1.0, limit / max(width, height))
    return max(1, int(width * factor)), max(1, int(height * factor))


class Images:
    """Deduplicated, bounded images of one snapshot: the Blender plug-in's
    budget (16 images, 16 MiB of pixels)."""

    def __init__(self):
        self.images, self.ids, self.bytes = [], {}, 0

    def mark(self):
        return len(self.images), dict(self.ids), self.bytes

    def rollback(self, mark):
        del self.images[mark[0]:]
        self.ids, self.bytes = mark[1], mark[2]

    def store(self, width, height, rgba):
        import base64
        if len(rgba) != width * height * 4:
            raise ValueError("Invalid image size")
        key = (width, height, rgba)
        if key not in self.ids:
            if self.bytes + len(rgba) > 16 * 1024 * 1024 or len(self.images) >= 16:
                raise ValueError("Live materials exceed 16 images or 16 MiB of pixels; reduce image sizes")
            self.bytes += len(rgba)
            self.ids[key] = len(self.images)
            self.images.append({"width": width, "height": height, "rgba": base64.b64encode(rgba).decode("ascii")})
        return self.ids[key]


def clamp_material(color, material):
    """Keep factors in the ranges the viewer accepts (appearance.rs): a
    Lambert colour of 1.2 or an emission weight of 100 would otherwise make
    it refuse the whole snapshot."""
    unit = lambda value: max(0.0, min(1.0, float(value))) if math.isfinite(float(value)) else 0.0
    material = dict(material)
    material["metallic"], material["roughness"] = unit(material["metallic"]), unit(material["roughness"])
    material["emissive"] = [max(0.0, min(64.0, float(v))) if math.isfinite(float(v)) else 0.0 for v in material["emissive"]]
    return [unit(value) for value in color], material


def default_material(color=(0.45, 0.22, 0.85, 1.0), metallic=0.0, roughness=0.6, double_sided=True):
    """The plain material of a face without a supported shader, as in the
    Blender plug-in."""
    return list(color), {"metallic": float(metallic), "roughness": float(roughness), "emissive": [0.0, 0.0, 0.0],
                         "double_sided": bool(double_sided), "alpha_mode": "OPAQUE", "textures": [None] * 4,
                         "transforms": [[0.0, 0.0, 1.0, 1.0, 0.0] for _ in range(4)]}


# --------------------------------------------------------------------- LODs

LOD_LEVELS = (("medium", 50.0), ("low", 25.0), ("lowest", 12.5))
LOD_MODES = ("fallback", "existing", "auto")
LOD_BUDGETS = ("ratio", "triangles")


def lod_default(level):
    ratio = dict(LOD_LEVELS)[level]
    return {"mode": "fallback", "object": "", "suffix": "_" + level, "budget": "ratio", "value": ratio}


def lod_settings_parse(text):
    """Per source id, per level: the LOD settings saved in the scene.

    The text is `id=level,level,level;id=...`, each level being
    `mode|object|suffix|budget|value`: a short Maya node name (namespace
    included) holds none of these separators, so the scene file needs no
    quoting.
    """
    result = {}
    for item in (text or "").split(";"):
        identity, _, levels = item.partition("=")
        if not identity or not levels:
            continue
        values = {}
        for (level, _ratio), spec in zip(LOD_LEVELS, levels.split(",")):
            fields = spec.split("|")
            value = lod_default(level)
            if len(fields) == 5 and fields[0] in LOD_MODES and fields[3] in LOD_BUDGETS:
                try:
                    number = float(fields[4])
                except ValueError:
                    number = value["value"]
                value.update(mode=fields[0], object=fields[1], suffix=fields[2], budget=fields[3],
                             value=number if math.isfinite(number) and number > 0 else value["value"])
            values[level] = value
        result[identity] = values
    return result


def lod_settings_format(settings):
    items = []
    for identity, levels in settings.items():
        specs = []
        for level, _ratio in LOD_LEVELS:
            spec = levels.get(level) or lod_default(level)
            specs.append("|".join((spec["mode"], spec["object"], spec["suffix"], spec["budget"], "{:g}".format(spec["value"]))))
        items.append(identity + "=" + ",".join(specs))
    return ";".join(items)


def lod_name(high_name, suffix):
    """The name a LOD mesh is looked up by: High's name without a trailing
    _high, .high or -high, then the level's suffix (Blender plug-in rule)."""
    import re
    return re.sub(r"[._-]high$", "", high_name, flags=re.IGNORECASE) + suffix


def lod_target(high_triangles, spec):
    if spec["budget"] == "triangles":
        return max(1, int(spec["value"]))
    return max(1, int(round(high_triangles * min(100.0, spec["value"]) / 100.0)))


def merge_palettes(meshes):
    """One joint palette for every LOD, as one upload needs: the Blender
    plug-in's rule. Plain wire meshes (vertex dicts) are remapped in place; a
    joint whose inverse bind differs between levels is refused."""
    names, binds = [], {}
    for mesh in meshes:
        for name, inverse in zip(mesh["joints"], mesh["inverse_bind"]):
            if name in binds and max(abs(a - b) for a, b in zip(inverse, binds[name])) > 0.0001:
                raise ValueError("LOD bind matrices disagree with High")
            if name not in binds:
                names.append(name)
                binds[name] = inverse
    if len(names) > MAX_JOINTS:
        raise ValueError("Import LODs exceed 110 avatar joints")
    for mesh in meshes:
        remap = [names.index(name) for name in mesh["joints"]]
        for face in mesh["faces"]:
            for vertex in face["vertices"]:
                vertex["joints"] = [remap[index] for index in vertex["joints"]] if remap else [0] * 4
        mesh["joints"], mesh["inverse_bind"] = list(names), [binds[name] for name in names]


def slot_mapping(level_groups, high_groups):
    """Each LOD surface's High surface, by shading group: a Maya shading
    group is one material slot."""
    mapping = []
    for group in level_groups:
        if group not in high_groups:
            raise ValueError("LOD materials must match High's shading groups")
        mapping.append(high_groups.index(group))
    if len(set(mapping)) != len(mapping):
        raise ValueError("Duplicate material slots in a LOD are ambiguous")
    return mapping


# ------------------------------------------------------------ explicit groups

def link_parents(static, hierarchy, overrides):
    """Each static source's linkset parent. `hierarchy` gives the nearest
    synchronized static Maya ancestor; `overrides` holds the list's explicit
    choices (a parent, or None for independent), which win, as the Blender
    plug-in's explicit groups win over Blender parenting."""
    parents = {}
    for source in static:
        if source in overrides:
            parent = overrides[source]
            parents[source] = parent if parent in static and parent != source else None
        else:
            parent = hierarchy.get(source)
            parents[source] = parent if parent in static else None
    return parents


def group(selected, active, overrides):
    """Group: the selected static sources join one linkset under `active`."""
    result = dict(overrides)
    result[active] = None
    for source in selected:
        if source != active:
            result[source] = active
    return result


def ungroup(selected, roots, overrides):
    """Ungroup: a selected root frees its whole linkset; a selected child
    leaves alone and the others stay linked."""
    result = dict(overrides)
    for source in selected:
        if roots.get(source) == source:
            for member, root in roots.items():
                if root == source:
                    result[member] = None
        elif source in roots:
            result[source] = None
    return result


def make_root(target, roots, overrides):
    """Make root: `target` becomes the root of its whole linkset."""
    result = dict(overrides)
    root = roots.get(target)
    for member, member_root in roots.items():
        if member_root == root and member != target:
            result[member] = target
    result[target] = None
    return result


def overrides_parse(text):
    result = {}
    for item in (text or "").split(";"):
        child, separator, parent = item.partition("=")
        if child and separator:
            result[child] = parent or None
    return result


def overrides_format(overrides):
    return ";".join("{}={}".format(child, parent or "") for child, parent in sorted(overrides.items()))


def shape_delta(new_inverse_bind, previous_inverse_bind, frame):
    """How one joint changes when a shape is applied, in Maya's frame:
    F^-1 · inverse(new inverse bind) · previous inverse bind · F (the Blender
    plug-in's shape_apply deltas). Both binds column-major, SL metres."""
    new_world = inverse(from_column_major(new_inverse_bind))
    return chain(inverse(frame), new_world, from_column_major(previous_inverse_bind), frame)
