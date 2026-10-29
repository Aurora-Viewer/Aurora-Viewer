"""Loopback-only transport. This worker never imports or accesses bpy."""

import json
import socket
import struct
import threading
import time
import zlib
import array
import base64
import sys

MAGIC = b"AURBLD1\n"
MAX_BYTES = 64 * 1024 * 1024
MAX_DECODED_BYTES = 256 * 1024 * 1024
COMPRESS_THRESHOLD = 1024 * 1024
MAX_REPLY = 8 * 1024 * 1024 + 4096


def wire_snapshot(snapshot, features):
    if "static-meshes" not in features and any(mesh.get("joints") == [] for mesh in snapshot.get("meshes", [])):
        raise ValueError("Update Aurora Viewer to preview non-rigged meshes")
    if not {"object-links", "source-ids"}.issubset(features) and any(mesh.get("link") for mesh in snapshot.get("meshes", [])):
        raise ValueError("Update Aurora Viewer to preview linked objects")
    if "meshes" not in snapshot:
        return snapshot
    meshes = snapshot["meshes"]
    if "source-ids" not in features or any("_slots" in mesh for mesh in meshes):
        # v1 peers without this optional capability reject unknown mesh fields.
        meshes = [{key: value for key, value in mesh.items() if key != "_slots" and (key != "source_id" or "source-ids" in features)}
                  for mesh in meshes]
    packed_geometry = "packed-geometry" in features
    buffered = any("_buffers" in face for mesh in meshes for face in mesh.get("faces", []))
    if packed_geometry or buffered:
        packed_meshes = []
        record = struct.Struct("<8f4B4f")
        for source in meshes:
            faces = []
            for face in source.get("faces", []):
                if "_buffers" in face:
                    vertices, index_bytes = face["_buffers"]
                else:
                    vertices = b"".join(record.pack(*v["position"], *v["normal"], *v["uv"], *v["joints"], *v["weights"])
                                        for v in face["vertices"])
                    indices = array.array("I", face["indices"])
                    if indices.itemsize != 4:
                        raise ValueError("Unsupported native index size")
                    if sys.byteorder != "little":
                        indices.byteswap()
                    index_bytes = indices.tobytes()
                packed = {key: value for key, value in face.items() if key not in {"vertices", "indices", "_buffers"}}
                if packed_geometry:
                    packed["packed"] = {"vertices": base64.b64encode(vertices).decode("ascii"),
                                        "indices": base64.b64encode(index_bytes).decode("ascii")}
                else:
                    # A reconnect may target an older viewer after capture.
                    packed["vertices"] = [{"position": list(v[:3]), "normal": list(v[3:6]), "uv": list(v[6:8]),
                                           "joints": list(v[8:12]), "weights": list(v[12:])} for v in record.iter_unpack(vertices)]
                    indices = array.array("I")
                    indices.frombytes(index_bytes)
                    if sys.byteorder != "little":
                        indices.byteswap()
                    packed["indices"] = indices.tolist()
                faces.append(packed)
            packed_meshes.append(dict(source, faces=faces))
        meshes = packed_meshes
    return snapshot if meshes is snapshot["meshes"] else dict(snapshot, meshes=meshes)


class SnapshotSizeError(ValueError):
    pass


def _encode(snapshot, hello):
    value = wire_snapshot(snapshot, hello.get("features", []))
    raw = json.dumps(value, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")
    limit = min(MAX_BYTES, hello.get("max_bytes", MAX_BYTES))
    compressed = "compressed-snapshots" in hello.get("features", [])
    if compressed and (len(raw) >= COMPRESS_THRESHOLD or len(raw) > limit):
        if len(raw) <= min(MAX_DECODED_BYTES, hello.get("max_decoded_bytes", MAX_DECODED_BYTES)):
            packed = b"ZLIB" + zlib.compress(raw, 1)
            if len(packed) < len(raw) and len(packed) <= limit:
                return packed
    if len(raw) <= limit:
        return raw
    raise SnapshotSizeError("This object exceeds the LiveMesh transfer limit; reduce its geometry or textures" if compressed else
                            "Update Aurora Viewer to preview this large object")


def _subset(snapshot, meshes):
    """Retain only used images and valid parent links without copying geometry."""
    images = snapshot.get("images", [])
    used = sorted({image for mesh in meshes for face in mesh.get("faces", [])
                   for image in (face.get("material") or {}).get("textures", []) if image is not None})
    remap = {image: index for index, image in enumerate(used)}
    identities = {mesh.get("source_id") for mesh in meshes}
    selected = []
    for source in meshes:
        mesh = dict(source)
        if mesh.get("link", {}).get("parent") not in identities:
            mesh.pop("link", None)
        faces = []
        for source_face in mesh.get("faces", []):
            face = dict(source_face)
            if face.get("material"):
                face["material"] = dict(face["material"])
                face["material"]["textures"] = [remap[image] if image is not None else None
                                                for image in face["material"].get("textures", [])]
            faces.append(face)
        mesh["faces"] = faces
        selected.append(mesh)
    return dict(snapshot, meshes=selected, images=[images[image] for image in used])


def prepare_frame(snapshot, hello):
    try:
        return _encode(snapshot, hello), {}
    except SnapshotSizeError:
        # A source that still exceeds the negotiated budget must not stop
        # otherwise valid objects. Retry only this exceptional oversized case.
        accepted, issues = [], {}
        payload = _encode(_subset(snapshot, []), hello)
        for mesh in snapshot.get("meshes", []):
            try:
                candidate = _encode(_subset(snapshot, accepted + [mesh]), hello)
            except SnapshotSizeError as error:
                issues[mesh.get("source_id", mesh["name"])] = str(error)
            else:
                accepted.append(mesh)
                payload = candidate
        return payload, issues


class Client:
    def __init__(self, port):
        self.port = port
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._wake = threading.Event()
        self._latest = None
        self._connection = None
        self._joints = frozenset()
        self._rest_axes = {}
        self._features = frozenset()
        self._shape = None
        self._shape_received = 0.0
        self._shape_result = None
        self._import = None
        self._import_received = 0.0
        self._import_result = None
        self._status = "Connecting to Aurora…"
        self._issues = (None, {})
        self._thread = threading.Thread(target=self._run, name="Aurora Live", daemon=True)
        self._thread.start()

    def state(self):
        with self._lock:
            return self._status, self._joints

    def submit(self, snapshot):
        with self._lock:
            self._latest = snapshot
        self._wake.set()

    def rest_axes(self):
        with self._lock:
            return self._rest_axes

    def features(self):
        with self._lock:
            return self._features

    def issues(self):
        with self._lock:
            return self._issues

    def take_shape(self, ready=True, pose=False):
        with self._lock:
            if self._shape is not None and time.monotonic() - self._shape_received > 30:
                identity = self._shape.get("id", "") if isinstance(self._shape, dict) else ""
                self._shape_result = {"type": "shape-result", "id": identity,
                                      "error": "Switch to Object Mode and send the shape again"}
                self._shape = None
                self._wake.set()
            apply_in_pose = pose and isinstance(self._shape, dict) and self._shape.get("action") == "apply"
            if not ready and not apply_in_pose:
                return None
            value, self._shape = self._shape, None
            return value

    def shape_result(self, identity, error=None):
        with self._lock:
            self._shape_result = {"type": "shape-result", "id": identity, "error": error}
        self._wake.set()

    def take_import(self, ready=True):
        with self._lock:
            if self._import is not None and time.monotonic() - self._import_received > 30:
                self._import_result = dict(self._import, error="Switch to Object Mode and click Import again")
                self._import = None
                self._wake.set()
            if not ready:
                return None
            request, self._import = self._import, None
            return request

    def import_result(self, result):
        with self._lock:
            self._import_result = result
        self._wake.set()

    def _reply(self, reader):
        reply = self._line(reader)
        if isinstance(reply.get("prepare_import"), dict):
            with self._lock:
                self._import = reply["prepare_import"]
                self._import_received = time.monotonic()
        if isinstance(reply, dict) and "shape" in reply:
            with self._lock:
                self._shape = reply["shape"]
                self._shape_received = time.monotonic()
        if isinstance(reply, dict) and reply.get("shape_cancel"):
            with self._lock:
                if isinstance(self._shape, dict) and self._shape.get("id") == reply["shape_cancel"]:
                    self._shape = None
        return reply

    def close(self):
        self._stop.set()
        self._wake.set()
        with self._lock:
            connection = self._connection
        if connection is not None:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def _set_status(self, message, joints=None):
        with self._lock:
            self._status = message
            if joints is not None:
                self._joints = frozenset(joints)

    @staticmethod
    def _line(stream):
        line = stream.readline(MAX_REPLY + 1)
        if not line or len(line) > MAX_REPLY or not line.endswith(b"\n"):
            raise ValueError("Invalid Aurora response")
        value = json.loads(line)
        if not isinstance(value, dict):
            raise ValueError("Invalid Aurora response")
        return value

    def _run(self):
        while not self._stop.is_set():
            try:
                with socket.create_connection(("127.0.0.1", self.port), timeout=1.0) as connection:
                    with self._lock:
                        self._connection = connection
                    if self._stop.is_set():
                        break
                    connection.settimeout(5.0)
                    connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    connection.sendall(MAGIC)
                    with connection.makefile("rb") as reader:
                        hello = self._line(reader)
                        if hello.get("protocol") != 1 or not hello.get("joints"):
                            raise ValueError("Incompatible Aurora protocol")
                        if "pbr-materials" not in hello.get("features", []):
                            raise ValueError("Update Aurora Viewer to use live PBR materials")
                        with self._lock:
                            self._rest_axes = hello.get("rest_axes", {})
                            self._features = frozenset(hello.get("features", []))
                        self._set_status("Connected to Aurora", hello["joints"])
                        if "shape-transfer" in hello.get("features", []):
                            features = [name for name in ("shape-transfer", "shape-apply", "import-lods") if name in hello.get("features", [])]
                            payload = json.dumps({"type": "features", "features": features}).encode("utf-8")
                            connection.sendall(struct.pack("<I", len(payload)) + payload)
                            self._reply(reader)
                        sent = None
                        heartbeat = time.monotonic()
                        self._wake.set()
                        while not self._stop.is_set():
                            self._wake.wait(max(0.0, 2.0 - (time.monotonic() - heartbeat)))
                            self._wake.clear()
                            if self._stop.is_set():
                                break
                            with self._lock:
                                latest = self._latest
                                result, self._shape_result = self._shape_result, None
                                prepared, self._import_result = self._import_result, None
                            if prepared is not None:
                                payload = b"LODS" + json.dumps(prepared, separators=(",", ":"), allow_nan=False).encode("utf-8")
                                if len(payload) > MAX_BYTES:
                                    payload = b"LODS" + json.dumps({"id": prepared["id"], "source": prepared["source"],
                                        "error": "Prepared LODs exceed 64 MiB"}, separators=(",", ":")).encode("utf-8")
                                # A full four-level decode can outlast an ordinary
                                # live frame. Keep it on the worker, bounded by the
                                # viewer's import deadline; close() still interrupts.
                                connection.settimeout(180.0)
                                try:
                                    connection.sendall(struct.pack("<I", len(payload)) + payload)
                                    self._reply(reader)
                                finally:
                                    connection.settimeout(5.0)
                                heartbeat = time.monotonic()
                            if result is not None:
                                payload = json.dumps(result, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
                                connection.sendall(struct.pack("<I", len(payload)) + payload)
                                self._reply(reader)
                                heartbeat = time.monotonic()
                            if latest is not None and latest["revision"] != sent:
                                payload, issues = prepare_frame(latest, hello)
                                with self._lock:
                                    self._issues = (latest["revision"], issues)
                                connection.sendall(struct.pack("<I", len(payload)) + payload)
                                reply = self._reply(reader)
                                sent = latest["revision"]
                                if reply.get("ok"):
                                    self._set_status("Connected · revision {}".format(sent))
                                else:
                                    self._set_status("Aurora: " + str(reply.get("error", "Snapshot rejected")))
                                    if reply.get("retry"):
                                        sent = None
                                        self._stop.wait(0.5)
                                        self._wake.set()
                                heartbeat = time.monotonic()
                            elif time.monotonic() - heartbeat > 2.0:
                                connection.sendall(b"\0\0\0\0")
                                self._reply(reader)
                                heartbeat = time.monotonic()
            except (OSError, ValueError, TypeError, OverflowError, struct.error) as error:
                self._set_status("Waiting for Aurora: " + str(error), ())
                self._stop.wait(1.0)
            finally:
                with self._lock:
                    self._connection = None
                    self._shape = None
                    self._shape_result = None
                    self._import = None
                    self._import_result = None
