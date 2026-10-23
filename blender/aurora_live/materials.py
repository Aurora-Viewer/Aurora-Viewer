"""Principled PBR to SL material inputs; bounded embedded images, no file paths."""
import base64
import math
import numpy as np
from .compat import principled_input

_cache = {}
MAX_IMAGE = 1024


def invalidate():
    _cache.clear()


def trees(material):
    pending, seen = [material.node_tree] if material and material.node_tree else [], set()
    while pending:
        tree = pending.pop()
        if tree.as_pointer() in seen:
            continue
        seen.add(tree.as_pointer())
        yield tree
        pending.extend(node.node_tree for node in tree.nodes if node.type == "GROUP" and node.node_tree)


def signature(objects):
    result, seen = [], set()
    for obj in objects:
        for slot in obj.material_slots:
            material = slot.material
            result.append(material.as_pointer() if material else 0)
            if not material or material.as_pointer() in seen:
                continue
            seen.add(material.as_pointer())
            result.append((tuple(material.diffuse_color), material.use_nodes, material.use_backface_culling))
            for tree in trees(material):
                for node in tree.nodes:
                    result.append((node.as_pointer(), node.type))
                    result.append(tuple(getattr(node, name, None) for name in
                                        ("vector_type", "interpolation", "projection", "extension", "uv_map", "space", "mode", "is_active_output")))
                    if node.type in {"VALUE", "RGB"}:
                        value = node.outputs[0].default_value
                        result.append(tuple(value) if hasattr(value, "__len__") else value)
                    image = getattr(node, "image", None)
                    if image:
                        result.append((image.as_pointer(), tuple(image.size), image.is_dirty, image.is_float,
                                       image.colorspace_settings.name, image.filepath_raw, image.alpha_mode))
                    for socket in node.inputs:
                        if hasattr(socket, "default_value"):
                            value = socket.default_value
                            result.append(tuple(value) if hasattr(value, "__len__") else value)
                result.extend((link.from_socket.as_pointer(), link.to_socket.as_pointer()) for link in tree.links)
    return tuple(result)


def _linked(socket):
    while socket.is_linked:
        link = socket.links[0]
        if link.from_node.type != "REROUTE":
            return link.from_node, link.from_socket.identifier
        socket = link.from_node.inputs[0]
    return None, None


def _value(socket):
    node, _output = _linked(socket)
    return node.outputs[0].default_value if node and node.type in {"VALUE", "RGB"} else socket.default_value


def _scalar(socket):
    value = _value(socket)
    return sum(v * weight for v, weight in zip(value[:3], (0.2126, 0.7152, 0.0722))) if hasattr(value, "__len__") else float(value)


def _color(socket):
    value = _value(socket)
    return list(value) if hasattr(value, "__len__") else [float(value)] * 3 + [1]


def _transform(node, uv_name=None):
    vector = node.inputs["Vector"]
    if not vector.is_linked:
        return [0, 0, 1, 1, 0]
    source, output = _linked(vector)
    def check_uv(node):
        if node.type == "UVMAP" and uv_name is not None and node.uv_map and node.uv_map != uv_name:
            raise ValueError("Live materials use the mesh's active UV map")
    if source.type in {"UVMAP", "TEX_COORD"} and (source.type == "UVMAP" or output == "UV"):
        check_uv(source)
        return [0, 0, 1, 1, 0]
    if source.type == "MAPPING" and source.vector_type == "POINT":
        parent, output = _linked(source.inputs["Vector"])
        if parent is None or parent.type not in {"UVMAP", "TEX_COORD"} or (parent.type == "TEX_COORD" and output != "UV"):
            raise ValueError("Use UV coordinates for live image textures")
        check_uv(parent)
        loc, rot, scale = (source.inputs[name].default_value for name in ("Location", "Rotation", "Scale"))
        if abs(rot.x) + abs(rot.y) > 0.000001:
            raise ValueError("Live UV mapping supports Z rotation only")
        angle = float(rot.z)
        return [loc.x - math.sin(angle) * scale.y, 1 - loc.y - math.cos(angle) * scale.y,
                float(scale.x), float(scale.y), angle]
    raise ValueError("Use a UV Image Texture, optionally through a Point Mapping node")


def _image(socket, normal=False, uv_name=None):
    node, output = _linked(socket)
    if node is None:
        return None
    if not normal and node.type in {"VALUE", "RGB"}:
        return None
    if normal:
        if (node.type != "NORMAL_MAP" or node.space != "TANGENT" or node.inputs["Strength"].is_linked
                or node.inputs["Strength"].default_value != 1):
            raise ValueError("Live normals need a tangent-space Normal Map with strength 1")
        if node.uv_map and uv_name is not None and node.uv_map != uv_name:
            raise ValueError("Live materials use the mesh's active UV map")
        node, output = _linked(node.inputs["Color"])
    channel = None
    if node and node.type in {"SEPARATE_COLOR", "SEPRGB"}:
        if node.type == "SEPARATE_COLOR" and node.mode != "RGB":
            raise ValueError("Bake procedural shaders to UV image textures for live materials")
        channel = {"Red": 0, "Green": 1, "Blue": 2, "R": 0, "G": 1, "B": 2}.get(output)
        node, output = _linked(node.inputs[0])
    if not node or node.type != "TEX_IMAGE" or node.image is None or node.extension != "REPEAT":
        raise ValueError("Bake procedural shaders to UV image textures for live materials")
    if node.projection != "FLAT" or node.image.source not in {"FILE", "GENERATED"}:
        raise ValueError("Live materials need flat, single-image UV textures")
    if node.interpolation != "Linear":
        raise ValueError("Use Linear interpolation for live image textures")
    if output == "Alpha":
        channel = 3
    return node.image, channel, _transform(node, uv_name), node.as_pointer()


def _pixels(image, color, use_alpha=False):
    key = (image.as_pointer(), color, use_alpha, tuple(image.size), image.colorspace_settings.name,
           image.is_dirty, image.is_float, image.filepath_raw, image.alpha_mode)
    if key in _cache:
        return _cache[key]
    width, height = image.size
    if image.colorspace_settings.name not in {"sRGB", "Non-Color", "Linear", "Linear Rec.709"}:
        raise ValueError("Use sRGB, Linear Rec.709 or Non-Color image color spaces")
    if not width or not height or width * height > 4096 * 4096:
        raise ValueError("Live source images must be loaded and at most 16 megapixels")
    values = np.empty(width * height * 4, dtype=np.float32)
    image.pixels.foreach_get(values)
    values = values.reshape((height, width, 4))
    factor = min(1, MAX_IMAGE / max(width, height))
    w, h = max(1, int(width * factor)), max(1, int(height * factor))
    values = values[np.arange(h) * height // h][:, np.arange(w) * width // w].copy()[::-1]
    rgb = values[:, :, :3]
    if use_alpha and image.is_float:
        # Blender float buffers are premultiplied; a used Alpha output makes
        # the Image Texture node return straight RGB. glTF images are straight.
        alpha = values[:, :, 3:4]
        rgb[:] = np.where(alpha > 0.000001, rgb / np.maximum(alpha, 0.000001), 0)
    # Float image buffers are already scene-linear, even with an sRGB source
    # tag. Byte buffers retain their source encoding (Blender Image API).
    srgb = image.colorspace_settings.name == "sRGB" and not image.is_float
    if color and not srgb:
        rgb[:] = np.where(rgb <= 0.0031308, rgb * 12.92, 1.055 * np.maximum(rgb, 0) ** (1/2.4) - 0.055)
    elif not color and srgb:
        rgb[:] = np.where(rgb <= 0.04045, rgb / 12.92, ((np.maximum(rgb, 0) + 0.055) / 1.055) ** 2.4)
    if not np.isfinite(values).all():
        raise ValueError("Image pixels must be finite")
    while _cache and sum(value.nbytes for value in _cache.values()) + values.nbytes > 64 * 1024 * 1024:
        _cache.pop(next(iter(_cache)))
    _cache[key] = np.clip(values, 0, 1)
    return _cache[key]


class Capture:
    def __init__(self):
        self.images, self.ids, self.bytes = [], {}, 0

    def store(self, value):
        raw = np.rint(np.clip(value, 0, 1) * 255).astype(np.uint8).tobytes()
        key = (value.shape, raw)
        if key not in self.ids:
            self.bytes += len(raw)
            if self.bytes > 16 * 1024 * 1024 or len(self.images) >= 16:
                raise ValueError("Live materials exceed 16 images or 16 MiB of pixels; reduce image sizes")
            self.ids[key] = len(self.images)
            self.images.append({"width": value.shape[1], "height": value.shape[0],
                                "rgba": base64.b64encode(raw).decode("ascii")})
        return self.ids[key]

    def material(self, source, slot):
        material = source.material_slots[slot].material if slot < len(source.material_slots) else None
        color = list(material.diffuse_color) if material else [0.45, 0.22, 0.85, 1]
        result = {"metallic": float(material.metallic) if material else 0.0,
                  "roughness": float(material.roughness) if material else 0.6, "emissive": [0, 0, 0],
                  "double_sided": not material or not material.use_backface_culling,
                  "alpha_mode": "OPAQUE", "textures": [None] * 4,
                  "transforms": [[0, 0, 1, 1, 0] for _ in range(4)]}
        shader = None
        if material and material.use_nodes and material.node_tree:
            output = next((n for n in material.node_tree.nodes if n.type == "OUTPUT_MATERIAL" and n.is_active_output), None)
            if output:
                shader, _ = _linked(output.inputs["Surface"])
            if shader is None or shader.type != "BSDF_PRINCIPLED":
                raise ValueError("Use a Principled BSDF surface for live PBR materials")
        if shader:
            for name in ("Subsurface Weight", "Coat Weight", "Sheen Weight", "Transmission Weight"):
                socket = principled_input(shader, name)
                if socket and (socket.is_linked or abs(float(socket.default_value)) > 0.000001):
                    raise ValueError("Bake advanced Principled effects to textures for Second Life PBR")
            uv_name = source.data.uv_layers.active.name if source.data.uv_layers.active else ""
            alpha = _image(shader.inputs["Alpha"], uv_name=uv_name)
            def texture(name, index, color_space=True, normal=False):
                spec = _image(principled_input(shader, name), normal, uv_name)
                if spec:
                    image, channel, transform, node = spec
                    value = _pixels(image, color_space, bool(alpha and alpha[3] == node))
                    if name == "Base Color" and not alpha:
                        value = value.copy()
                        value[:, :, 3] = 1
                    if channel is not None:
                        value = np.repeat(value[:, :, channel:channel+1], 4, axis=2)
                        if color_space and channel == 3:
                            rgb = value[:, :, :3]
                            rgb[:] = np.where(rgb <= 0.0031308, rgb * 12.92, 1.055 * np.maximum(rgb, 0) ** (1/2.4) - 0.055)
                    result["textures"][index] = self.store(value)
                    result["transforms"][index] = transform
                return spec
            base = texture("Base Color", 0)
            color = [1, 1, 1, 1] if base else _color(shader.inputs["Base Color"])
            if alpha:
                if not base or alpha[0] is not base[0] or alpha[1] != 3 or alpha[2] != base[2]:
                    raise ValueError("Connect the base image's Alpha output directly to Principled Alpha")
            else:
                color[3] = _scalar(shader.inputs["Alpha"])
            if color[3] < 0.999 or alpha:
                result["alpha_mode"] = "BLEND"
            texture("Normal", 1, False, True)
            rough, metal = _image(shader.inputs["Roughness"], uv_name=uv_name), _image(shader.inputs["Metallic"], uv_name=uv_name)
            result["roughness"] = 1.0 if rough else _scalar(shader.inputs["Roughness"])
            result["metallic"] = 1.0 if metal else _scalar(shader.inputs["Metallic"])
            if rough or metal:
                specs = [spec for spec in (rough, metal) if spec]
                if any(spec[2] != specs[0][2] for spec in specs):
                    raise ValueError("Roughness and metallic images must share a UV transform")
                arrays = [_pixels(spec[0], False, bool(alpha and alpha[3] == spec[3])) for spec in specs]
                h, w = max(a.shape[0] for a in arrays), max(a.shape[1] for a in arrays)
                orm = np.ones((h, w, 4), dtype=np.float32)
                for spec, channel in ((rough, 1), (metal, 2)):
                    if spec:
                        value = _pixels(spec[0], False, bool(alpha and alpha[3] == spec[3]))
                        value = value[np.arange(h) * value.shape[0] // h][:, np.arange(w) * value.shape[1] // w]
                        orm[:, :, channel] = (value[:, :, spec[1]] if spec[1] is not None else
                                              np.einsum("ijk,k->ij", value[:, :, :3], np.array((0.2126, 0.7152, 0.0722))))
                result["textures"][2] = self.store(orm)
                result["transforms"][2] = specs[0][2]
            emission = texture("Emission Color", 3)
            strength = _scalar(shader.inputs["Emission Strength"])
            result["emissive"] = [strength * float(v) for v in ([1, 1, 1] if emission else _color(principled_input(shader, "Emission Color"))[:3])]
            if any(value is not None for value in result["textures"]) and not uv_name:
                raise ValueError("Add an active UV map to use live image textures")
        if color[3] < 0.999:
            result["alpha_mode"] = "BLEND"
        return color, result
