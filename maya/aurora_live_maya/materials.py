"""Maya shaders to the LiveMesh PBR material; images read from file textures.

The rules follow the Blender plug-in (blender/aurora_live/materials.py): a
material it cannot read is shown in its plain colour, with the reason kept as
a warning; image textures only, at most 1024 pixels, sRGB colour maps and
linear data maps. Supported shaders: Standard Surface, aiStandardSurface, USD
Preview Surface, and Lambert, Blinn and Phong as diffuse colour.
"""

import os

from maya import cmds

from . import convert

PBR = {
    # Shader: base weight, base colour, metalness, roughness, emission weight,
    # emission colour, opacity, normal; then the effects that need baking.
    "standardSurface": ("base", "baseColor", "metalness", "specularRoughness", "emission", "emissionColor", "opacity",
                        "normalCamera", ("subsurface", "coat", "sheen", "transmission")),
    "aiStandardSurface": ("base", "baseColor", "metalness", "specularRoughness", "emission", "emissionColor", "opacity",
                          "normalCamera", ("subsurface", "coat", "sheen", "transmission")),
    "usdPreviewSurface": (None, "diffuseColor", "metallic", "roughness", None, "emissiveColor", "opacity", "normal", ()),
}
CLASSIC = {"lambert": 1.0, "blinn": 0.5, "phong": 0.5, "phongE": 0.5}
UNSUPPORTED = "Use Standard Surface, aiStandardSurface, USD Preview Surface, Lambert, Blinn or Phong for live materials"
BAKE = "Bake procedural shaders to file textures for live materials"

SRGB_SPACES = {"sRGB", "sRGB - Texture", "Utility - sRGB - Texture", "sRGB Encoded Rec.709 (sRGB)", "srgb_texture",
               "Output - sRGB", "sRGB gamma"}
LINEAR_SPACES = {"Raw", "raw", "Utility - Raw", "Linear sRGB", "scene-linear Rec.709-sRGB", "Linear Rec.709 (sRGB)",
                 "Utility - Linear - sRGB", "lin_rec709", "lin_srgb", "Data", "Non-Color"}

# (path, mtime, size) -> (width, height, RGBA8 top row first, has alpha)
_cache = {}


def invalidate():
    _cache.clear()


def history(group):
    """Every node of a shading group's network, for change tracking."""
    if not group or not cmds.objExists(group):
        return []
    return [group] + [node for node in cmds.listHistory(group, pruneDagObjects=True) or [] if node != group]


def _attr(node, name):
    return node + "." + name


def _has(node, name):
    return bool(name) and cmds.attributeQuery(name, node=node, exists=True)


def _get(node, name, default=0.0):
    if not _has(node, name):
        return default
    value = cmds.getAttr(_attr(node, name))
    if isinstance(value, list) and value and isinstance(value[0], tuple):
        value = value[0]
    return value


def _source(node, name):
    """The (node, output attribute) plugged into an attribute or one of its
    children, or None; conversion nodes are skipped."""
    if not _has(node, name):
        return None
    children = cmds.attributeQuery(name, node=node, listChildren=True) or []
    for attribute in [name] + children:
        plugs = cmds.listConnections(_attr(node, attribute), source=True, destination=False, plugs=True,
                                     skipConversionNodes=True) or []
        if plugs:
            source, output = plugs[0].split(".", 1)
            return source, output
    return None


def _check_placement(file_node):
    """The UV transform of a file texture's place2dTexture, if any."""
    placement = _source(file_node, "uvCoord")
    if placement is None:
        return convert.uv_transform()
    node = placement[0]
    if cmds.nodeType(node) != "place2dTexture":
        raise ValueError("Live image textures need a plain 2D placement")
    if (any(abs(value) > 1e-6 for value in (_get(node, "rotateUV"), _get(node, "rotateFrame"),
                                            *_get(node, "translateFrame", (0, 0)), *_get(node, "noiseUV", (0, 0))))
            or any(abs(value - 1) > 1e-6 for value in _get(node, "coverage", (1, 1)))
            or _get(node, "mirrorU", False) or _get(node, "mirrorV", False) or _get(node, "stagger", False)):
        raise ValueError("Live image textures support repeat and offset only")
    return convert.uv_transform(_get(node, "repeatUV", (1, 1)), _get(node, "offset", (0, 0)))


def _file(node, name):
    """(file node, output) feeding an attribute, None when it is unplugged;
    any other texture node is refused."""
    source = _source(node, name)
    if source is None:
        return None
    if cmds.nodeType(source[0]) not in ("file", "aiImage"):
        raise ValueError(BAKE)
    return source


def _path(file_node):
    if cmds.nodeType(file_node) == "aiImage":
        path = cmds.getAttr(file_node + ".filename")
    else:
        if _get(file_node, "uvTilingMode", 0):
            raise ValueError("UDIM and tiled textures are not supported live; use one image")
        path = cmds.getAttr(file_node + ".computedFileTextureNamePattern") or cmds.getAttr(file_node + ".fileTextureName")
    path = (path or "").strip()
    if not path or not os.path.isfile(path):
        raise ValueError("Texture file not found: " + os.path.basename(path or "?"))
    return path


def _space(file_node):
    space = _get(file_node, "colorSpace", "sRGB") or "sRGB"
    if space in SRGB_SPACES:
        return "srgb"
    if space in LINEAR_SPACES:
        return "linear"
    raise ValueError("Use sRGB or Raw colour spaces for live textures (found {})".format(space))


def _load(path, limit=1024, size=None):
    """RGBA8, top row first, scaled to at most `limit` pixels (or to `size`)."""
    key = (path, os.path.getmtime(path), limit, size)
    if key in _cache:
        return _cache[key]
    image = _qt_image(path)
    if image is None:
        raise ValueError("Cannot read texture " + os.path.basename(path) + "; use PNG, JPEG, TGA, TIFF or BMP")
    width, height, rgba, alpha = image
    target = size or convert.fit(width, height, limit)
    if target != (width, height):
        rgba = _resample(rgba, width, height, *target)
        width, height = target
    while _cache and sum(len(value[2]) for value in _cache.values()) + len(rgba) > 64 * 1024 * 1024:
        _cache.pop(next(iter(_cache)))
    _cache[key] = (width, height, rgba, alpha)
    return _cache[key]


def _resample(rgba, width, height, new_width, new_height):
    """Nearest sampling, as the Blender plug-in scales its images."""
    rows = []
    stride = width * 4
    columns = [x * width // new_width * 4 for x in range(new_width)]
    for y in range(new_height):
        source = rgba[(y * height // new_height) * stride:][:stride]
        rows.append(b"".join(source[x:x + 4] for x in columns))
    return b"".join(rows)


def _qt_image(path):
    """Read with Qt, which ships with Maya; QImage rows start at the top."""
    try:
        from PySide6.QtGui import QImage
    except ImportError:
        try:
            from PySide2.QtGui import QImage
        except ImportError:
            return None
    image = QImage(path)
    if image.isNull():
        return None
    alpha = image.hasAlphaChannel()
    image = image.convertToFormat(QImage.Format_RGBA8888)
    width, height = image.width(), image.height()
    if width * height > 8192 * 8192:
        raise ValueError("Live source images must be at most 64 megapixels")
    bits = image.constBits()
    if hasattr(bits, "setsize"):
        bits.setsize(image.sizeInBytes() if hasattr(image, "sizeInBytes") else image.byteCount())
    raw = bytes(bits)
    stride = image.bytesPerLine()
    if stride != width * 4:
        raw = b"".join(raw[row * stride:row * stride + width * 4] for row in range(height))
    return width, height, raw[:width * height * 4], alpha


def _channel_index(output):
    if output in ("outAlpha", "outTransparency", "outTransparencyR", "outTransparencyG", "outTransparencyB"):
        return 3
    return {"outColorR": 0, "outColorG": 1, "outColorB": 2}.get(output)


def _texture(spec, color_space):
    """(width, height, RGBA8, transform, file node) of a file texture, in the
    encoding the viewer expects: sRGB for colour maps, linear for data."""
    file_node, output = spec
    width, height, rgba, alpha = _load(_path(file_node))
    space = _space(file_node) if cmds.nodeType(file_node) == "file" else ("srgb" if color_space else "linear")
    if color_space and space == "linear":
        rgba = convert.recode(rgba, convert.TO_SRGB)
    elif not color_space and space == "srgb":
        rgba = convert.recode(rgba, convert.TO_LINEAR)
    index = _channel_index(output)
    if index is not None:
        rgba = convert.channel(rgba, index)
    transform = _check_placement(file_node) if cmds.nodeType(file_node) == "file" else convert.uv_transform()
    return width, height, rgba, transform, file_node, alpha


def _normal(node, name):
    """A tangent-space normal map: bump2d set to Tangent Space Normals with a
    depth of 1, or aiNormalMap at strength 1, fed by a file texture."""
    source = _source(node, name)
    if source is None:
        return None
    kind = cmds.nodeType(source[0])
    if kind in ("file", "aiImage"):
        return source
    if kind == "bump2d":
        if _get(source[0], "bumpInterp", 0) != 1 or abs(_get(source[0], "bumpDepth", 1) - 1) > 1e-6:
            raise ValueError("Live normals need a bump2d set to Tangent Space Normals with depth 1")
        spec = _file(source[0], "bumpValue")
    elif kind == "aiNormalMap":
        if (abs(_get(source[0], "strength", 1) - 1) > 1e-6 or _get(source[0], "invertX", False)
                or _get(source[0], "invertY", False)):
            raise ValueError("Live normals need an aiNormalMap with strength 1 and no inversion")
        spec = _file(source[0], "input")
    else:
        raise ValueError("Live normals need a tangent-space normal map")
    if spec is None:
        raise ValueError("Live normals need a tangent-space normal map")
    return spec[0], "outColor"


def _scalar(node, name, default):
    value = _get(node, name, default)
    if isinstance(value, (tuple, list)):
        value = sum(float(v) * w for v, w in zip(value[:3], (0.2126, 0.7152, 0.0722)))
    return float(value)


def _color(node, name, default=(1.0, 1.0, 1.0)):
    value = _get(node, name, default)
    if not isinstance(value, (tuple, list)):
        value = (value, value, value)
    return [float(v) for v in value[:3]]


def capture(group, images, warnings, double_sided=True, has_uvs=True):
    """(linear base colour factor, material) of one shading group."""
    mark = images.mark()
    try:
        return _capture(group, images, double_sided, has_uvs)
    except ValueError as error:
        images.rollback(mark)
        warnings.append(str(error))
        return _plain(group, double_sided)


def _shader(group):
    if not group or not cmds.objExists(group):
        return None
    source = _source(group, "surfaceShader")
    return source[0] if source else None


def _plain(group, double_sided):
    """The shader's colour alone, or the default material."""
    shader = _shader(group)
    if shader is None:
        return convert.default_material(double_sided=double_sided)
    kind = cmds.nodeType(shader)
    try:
        if kind in PBR:
            weight, color_name = PBR[kind][0], PBR[kind][1]
            factor = _scalar(shader, weight, 1.0) if weight else 1.0
            color = [factor * value for value in _color(shader, color_name)] + [1.0]
            return convert.default_material(color, double_sided=double_sided)
        if kind in CLASSIC:
            return convert.default_material(_color(shader, "color", (0.5, 0.5, 0.5)) + [1.0], 0.0, CLASSIC[kind],
                                            double_sided)
    except (ValueError, RuntimeError):
        pass
    return convert.default_material(double_sided=double_sided)


def _capture(group, images, double_sided, has_uvs):
    shader = _shader(group)
    if shader is None:
        return convert.default_material(double_sided=double_sided)
    kind = cmds.nodeType(shader)
    color, material = convert.default_material(double_sided=double_sided)
    textures, transforms = material["textures"], material["transforms"]
    if kind in PBR:
        weight, base, metal, rough, emission, emission_color, opacity, normal, advanced = PBR[kind]
        for effect in advanced:
            if _has(shader, effect) and (_source(shader, effect) or abs(_scalar(shader, effect, 0.0)) > 1e-6):
                raise ValueError("Bake subsurface, coat, sheen and transmission to textures for Second Life PBR")
        factor = _scalar(shader, weight, 1.0) if weight else 1.0
        base_spec = _file(shader, base)
        opacity_spec = _file(shader, opacity)
        normal_spec = _normal(shader, normal)
        rough_spec, metal_spec = _file(shader, rough), _file(shader, metal)
        emission_spec = _file(shader, emission_color)
        emission_weight = _scalar(shader, emission, 1.0) if emission else 1.0
        roughness_value, metallic_value = _scalar(shader, rough, 0.5), _scalar(shader, metal, 0.0)
        opacity_value = _scalar(shader, opacity, 1.0)
        emissive = _color(shader, emission_color, (0.0, 0.0, 0.0))
        base_color = _color(shader, base)
    elif kind in CLASSIC:
        factor = 1.0
        base_spec, normal_spec = _file(shader, "color"), _normal(shader, "normalCamera")
        opacity_spec = _file(shader, "transparency")
        rough_spec = metal_spec = None
        emission_spec, emission_weight = _file(shader, "incandescence"), 1.0
        roughness_value, metallic_value = CLASSIC[kind], 0.0
        opacity_value = 1.0 - _scalar(shader, "transparency", 0.0)
        emissive = _color(shader, "incandescence", (0.0, 0.0, 0.0))
        base_color = _color(shader, "color", (0.5, 0.5, 0.5))
    else:
        raise ValueError(UNSUPPORTED)
    if not has_uvs and any((base_spec, normal_spec, rough_spec, metal_spec, emission_spec)):
        raise ValueError("Add a UV set to use live image textures")
    material["roughness"], material["metallic"] = roughness_value, metallic_value
    if base_spec:
        width, height, rgba, transform, file_node, has_alpha = _texture((base_spec[0], "outColor"), True)
        if opacity_spec:
            if opacity_spec[0] != file_node or _channel_index(opacity_spec[1]) != 3:
                raise ValueError("Connect the base colour file's alpha to the opacity")
            material["alpha_mode"] = "BLEND"
        else:
            rgba = convert.opaque(rgba)
        textures[0], transforms[0] = images.store(width, height, rgba), transform
        color = [factor, factor, factor, 1.0]
    else:
        if opacity_spec:
            raise ValueError("Connect the base colour file's alpha to the opacity")
        color = [factor * value for value in base_color] + [max(0.0, min(1.0, opacity_value))]
    if normal_spec:
        width, height, rgba, transform, _node, _alpha = _texture(normal_spec, False)
        textures[1], transforms[1] = images.store(width, height, convert.opaque(rgba)), transform
    if rough_spec or metal_spec:
        specs = [spec for spec in (rough_spec, metal_spec) if spec]
        loaded = [_texture(spec, False) for spec in specs]
        if any(item[3] != loaded[0][3] for item in loaded):
            raise ValueError("Roughness and metalness images must share a UV placement")
        size = (max(item[0] for item in loaded), max(item[1] for item in loaded))
        resized = []
        for item in loaded:
            rgba = item[2] if (item[0], item[1]) == size else _resample(item[2], item[0], item[1], *size)
            resized.append((rgba, 0))
        pairs = iter(resized)
        roughness = next(pairs) if rough_spec else None
        metallic = next(pairs) if metal_spec else None
        textures[2] = images.store(size[0], size[1], convert.orm(size, roughness, metallic))
        transforms[2] = loaded[0][3]
        if rough_spec:
            material["roughness"] = 1.0
        if metal_spec:
            material["metallic"] = 1.0
    if emission_spec:
        width, height, rgba, transform, _node, _alpha = _texture(emission_spec, True)
        textures[3], transforms[3] = images.store(width, height, convert.opaque(rgba)), transform
        material["emissive"] = [emission_weight] * 3
    else:
        material["emissive"] = [emission_weight * value for value in emissive]
    if color[3] < 0.999:
        material["alpha_mode"] = "BLEND"
    return color, material
