"""Transport tests without importing Blender or opening external connections."""
import importlib.util
import io
import json
from pathlib import Path
import socket
import struct
import threading
import time
import unittest
from unittest.mock import patch
import zlib

spec = importlib.util.spec_from_file_location("aurora_client", Path(__file__).resolve().parents[1] / "aurora_live" / "client.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def receive(connection, size):
    value = b""
    while len(value) < size:
        chunk = connection.recv(size - len(value))
        if not chunk:
            raise EOFError("Incomplete message")
        value += chunk
    return value


def hello(connection):
    if receive(connection, 8) != module.MAGIC:
        raise ValueError("Bad handshake")
    connection.sendall(b'{"protocol":1,"joints":["mPelvis"],"max_bytes":67108864,"features":["pbr-materials"]}\n')


def snapshot(connection):
    length = struct.unpack("<I", receive(connection, 4))[0]
    return json.loads(receive(connection, length))


class ClientTests(unittest.TestCase):
    def test_detached_buffers_support_packed_and_older_peers_without_mutation(self):
        vertex = struct.pack("<8f4B4f", 1, 2, 3, 0, 0, 1, 0.25, 0.75, 0, 0, 0, 0, 1, 0, 0, 0)
        value = {"protocol": 1, "revision": 1, "meshes": [{"name": "Detached capture", "source_id": "source",
                 "faces": [{"color": [1, 1, 1, 1], "_buffers": (vertex, struct.pack("<3I", 0, 0, 0))}]}]}
        packed = module.wire_snapshot(value, ["pbr-materials", "source-ids", "packed-geometry"])
        self.assertEqual(module.base64.b64decode(packed["meshes"][0]["faces"][0]["packed"]["vertices"]), vertex)
        plain = module.wire_snapshot(value, ["pbr-materials", "source-ids"])
        face = plain["meshes"][0]["faces"][0]
        self.assertEqual(face["vertices"][0]["position"], [1, 2, 3])
        self.assertEqual(face["vertices"][0]["weights"], [1, 0, 0, 0])
        self.assertEqual(face["indices"], [0, 0, 0])
        self.assertIn("_buffers", value["meshes"][0]["faces"][0])
        self.assertNotIn("_buffers", face)

    def test_packed_geometry_uses_the_same_f32_values_and_preserves_the_capture(self):
        vertex = {"position": [0.123456789, -2, 3], "normal": [0, 1, 0], "uv": [0.25, 0.75],
                  "joints": [1, 2, 3, 0], "weights": [0.17, 0.28, 0.55, 0]}
        value = {"protocol": 1, "revision": 1, "meshes": [{"name": "Packed source", "source_id": "source",
                 "faces": [{"color": [1, 1, 1, 1], "vertices": [vertex], "indices": [0, 0, 0]}]}]}
        original = json.dumps(value)
        packed = module.wire_snapshot(value, ["pbr-materials", "source-ids", "packed-geometry"])
        face = packed["meshes"][0]["faces"][0]
        expected = struct.pack("<8f4B4f", *vertex["position"], *vertex["normal"], *vertex["uv"], *vertex["joints"], *vertex["weights"])
        self.assertEqual(module.base64.b64decode(face["packed"]["vertices"]), expected)
        self.assertEqual(module.base64.b64decode(face["packed"]["indices"]), struct.pack("<3I", 0, 0, 0))
        self.assertNotIn("vertices", face)
        self.assertEqual(original, json.dumps(value))
        self.assertEqual(module.wire_snapshot(value, ["pbr-materials", "source-ids"]), value)

    def test_compressed_snapshots_preserve_all_data_and_require_negotiation(self):
        value = {"protocol": 1, "revision": 4, "meshes": [{"name": "Detailed source", "faces": [], "padding": "mesh data " * 2000}]}
        original = json.dumps(value, separators=(",", ":")).encode()
        with patch.object(module, "MAX_BYTES", 2048):
            payload, errors = module.prepare_frame(value, {"features": ["pbr-materials", "compressed-snapshots"]})
            self.assertTrue(payload.startswith(b"ZLIB"))
            self.assertEqual(json.loads(zlib.decompress(payload[4:])), value)
            self.assertFalse(errors)
            self.assertGreater(len(original), module.MAX_BYTES)
            self.assertLess(len(payload), module.MAX_BYTES)
            plain, errors = module.prepare_frame(value, {"features": ["pbr-materials"]})
            self.assertFalse(json.loads(plain)["meshes"])
            self.assertIn("Update Aurora", errors["Detailed source"])

    def test_oversized_source_is_omitted_with_images_and_links_repaired(self):
        big = {"name": "Too large", "source_id": "root", "faces": [], "padding": "x" * 4096}
        small = {"name": "Valid child", "source_id": "child", "link": {"parent": "root", "offset": [0, 0, 0]},
                 "faces": [{"material": {"textures": [1, None, None, None]}, "vertices": [], "indices": []}]}
        value = {"protocol": 1, "revision": 1, "meshes": [big, small], "images": ["unused" * 200, "used"]}
        with patch.object(module, "MAX_BYTES", 1024):
            payload, errors = module.prepare_frame(value, {"features": ["pbr-materials", "source-ids", "object-links"]})
        sent = json.loads(payload)
        self.assertEqual(set(errors), {"root"})
        self.assertEqual([mesh["source_id"] for mesh in sent["meshes"]], ["child"])
        self.assertNotIn("link", sent["meshes"][0])
        self.assertEqual(sent["images"], ["used"])
        self.assertEqual(sent["meshes"][0]["faces"][0]["material"]["textures"], [0, None, None, None])
        self.assertIn("link", small)
        self.assertEqual(small["faces"][0]["material"]["textures"][0], 1)

    def test_decoded_budget_is_enforced_even_when_compression_is_small(self):
        value = {"protocol": 1, "revision": 1, "meshes": [{"name": "Over decoded budget", "padding": "x" * 8000}]}
        hello_value = {"features": ["pbr-materials", "compressed-snapshots"], "max_bytes": 1024, "max_decoded_bytes": 4096}
        payload, errors = module.prepare_frame(value, hello_value)
        self.assertFalse(json.loads(payload)["meshes"])
        self.assertIn("transfer limit", errors["Over decoded budget"])

    def test_socket_sends_compressed_frames_then_recovers_from_an_oversized_source(self):
        received = []
        first = threading.Event()
        complete = threading.Event()

        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(5)
                self.assertEqual(receive(connection, 8), module.MAGIC)
                connection.sendall(b'{"protocol":1,"joints":["mPelvis"],"max_bytes":1024,"max_decoded_bytes":4096,"features":["pbr-materials","compressed-snapshots"]}\n')
                received.append(snapshot(connection))
                connection.sendall(b'{"ok":true,"revision":1}\n')
                first.set()
                length = struct.unpack("<I", receive(connection, 4))[0]
                data = receive(connection, length)
                self.assertTrue(data.startswith(b"ZLIB"))
                received.append(json.loads(zlib.decompress(data[4:])))
                connection.sendall(b'{"ok":true,"revision":2}\n')
                complete.set()

        client, thread, failures = self.serve(server)
        client.submit({"protocol": 1, "revision": 1, "meshes": [{"name": "Too large", "padding": "x" * 8000}]})
        self.assertTrue(first.wait(3))
        self.assertIn("Too large", client.issues()[1])
        client.submit({"protocol": 1, "revision": 2, "meshes": [{"name": "Repaired", "padding": "x" * 3000}]})
        self.assertTrue(complete.wait(3))
        self.assertFalse(client.issues()[1])
        self.finish(client, thread, failures)
        self.assertFalse(received[0]["meshes"])
        self.assertEqual(received[1]["meshes"][0]["name"], "Repaired")

    def test_import_request_and_binary_result_are_independent_of_live_revisions(self):
        identity = {"id": "00000000-0000-0000-0000-000000000123", "source": "00000000-0000-0000-0000-000000000456"}
        received = []
        ready = threading.Event()
        decoding, release = threading.Event(), threading.Event()
        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(5)
                self.assertEqual(receive(connection, 8), module.MAGIC)
                connection.sendall(b'{"protocol":1,"joints":["mPelvis"],"features":["pbr-materials","shape-transfer","import-lods"]}\n')
                received.append(snapshot(connection))
                connection.sendall(json.dumps({"ok": True, "prepare_import": identity}).encode() + b"\n")
                ready.set()
                size = struct.unpack("<I", receive(connection, 4))[0]
                payload = receive(connection, size)
                self.assertEqual(payload[:4], b"LODS")
                received.append(json.loads(payload[4:]))
                decoding.set()
                self.assertTrue(release.wait(3))
                connection.sendall(b'{"ok":true}\n')
                received.append(snapshot(connection))
                connection.sendall(b'{"ok":true,"revision":4}\n')
        client, thread, failures = self.serve(server)
        self.assertTrue(ready.wait(3))
        self.assertIsNone(client.take_import(ready=False))
        deadline, request = time.monotonic() + 3, None
        while request is None and time.monotonic() < deadline:
            request = client.take_import()
            time.sleep(0.01)
        self.assertEqual(request, identity)
        self.assertIsNone(client.take_import())
        client.import_result(dict(identity, levels=[], images=[]))
        self.assertTrue(decoding.wait(3))
        self.assertEqual(client._connection.gettimeout(), 180.0)
        release.set()
        client.submit({"protocol": 1, "revision": 4, "meshes": [], "images": []})
        self.finish(client, thread, failures)
        self.assertIn("import-lods", received[0]["features"])
        self.assertEqual(received[1]["id"], identity["id"])
        self.assertEqual(received[2]["revision"], 4)

    def test_import_waiting_for_object_mode_expires_with_an_actionable_error(self):
        client = module.Client.__new__(module.Client)
        client._lock, client._wake = threading.Lock(), threading.Event()
        client._import = {"id": "request", "source": "source"}
        client._import_received = time.monotonic() - 31
        self.assertIsNone(client.take_import(ready=False))
        self.assertIn("Object Mode", client._import_result["error"])
        self.assertIsNone(client.take_import())

    def test_shape_waiting_for_object_mode_expires_and_can_be_cancelled(self):
        client = module.Client.__new__(module.Client)
        client._lock = threading.Lock()
        client._wake = threading.Event()
        client._shape = {"id": "test-shape"}
        client._shape_received = time.monotonic() - 31
        client._shape_result = None
        self.assertIsNone(client.take_shape(ready=False))
        self.assertIsNone(client.take_shape())
        self.assertEqual(client._shape_result["id"], "test-shape")
        self.assertIsNotNone(client._shape_result["error"])
        client._shape = {"id": "test-shape"}
        client._shape_received = time.monotonic()
        client._reply(io.BytesIO(b'{"ok":true,"shape_cancel":"test-shape"}\n'))
        self.assertIsNone(client.take_shape())

    def test_pose_mode_only_receives_apply_and_keeps_creation_pending(self):
        client = module.Client.__new__(module.Client)
        client._lock = threading.Lock()
        client._wake = threading.Event()
        client._shape = {"id": "test-shape", "action": "create"}
        client._shape_received = time.monotonic()
        self.assertIsNone(client.take_shape(ready=False, pose=True))
        client._shape["action"] = "apply"
        self.assertEqual(client.take_shape(ready=False, pose=True)["action"], "apply")

    def test_shape_transfer_negotiates_receives_large_data_and_confirms_main_thread_import(self):
        received = []
        identity = "00000000-0000-0000-0000-000000000123"
        ready = threading.Event()

        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(5)
                self.assertEqual(receive(connection, 8), module.MAGIC)
                connection.sendall(b'{"protocol":1,"joints":["mPelvis"],"features":["pbr-materials","shape-transfer","shape-apply"]}\n')
                received.append(snapshot(connection))
                payload = {"id": identity, "test_data": "x" * 300000}
                connection.sendall(json.dumps({"ok": True, "shape": payload}).encode() + b"\n")
                ready.set()
                received.append(snapshot(connection))
                connection.sendall(b'{"ok":true}\n')

        client, thread, failures = self.serve(server)
        self.assertTrue(ready.wait(3))
        deadline = time.monotonic() + 3
        payload = None
        while payload is None and time.monotonic() < deadline:
            payload = client.take_shape()
            time.sleep(0.01)
        self.assertEqual(payload["id"], identity)
        self.assertEqual(len(payload["test_data"]), 300000)
        self.assertIsNone(client.take_shape(), "one import per delivered request")
        client.shape_result(identity)
        self.finish(client, thread, failures)
        self.assertEqual(received[0], {"type": "features", "features": ["shape-transfer", "shape-apply"]})
        self.assertEqual(received[1], {"type": "shape-result", "id": identity, "error": None})

    def test_source_identity_is_negotiated_without_mutating_the_capture(self):
        value = {"revision": 1, "meshes": [{"name": "Shirt", "source_id": "stable-id"}]}
        self.assertIs(module.wire_snapshot(value, ["pbr-materials", "source-ids"]), value)
        legacy = module.wire_snapshot(value, ["pbr-materials"])
        self.assertEqual(legacy, {"revision": 1, "meshes": [{"name": "Shirt"}]})
        self.assertEqual(value["meshes"][0]["source_id"], "stable-id")

    def test_busy_image_staging_retries_without_another_edit(self):
        received = []
        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(4)
                hello(connection)
                received.append(snapshot(connection)["revision"])
                connection.sendall(b'{"ok":false,"retry":true,"error":"staging busy"}\n')
                received.append(snapshot(connection)["revision"])
                connection.sendall(b'{"ok":true,"revision":7}\n')
        client, thread, failures = self.serve(server)
        client.submit({"revision":7})
        self.finish(client, thread, failures)
        self.assertEqual(received, [7,7])

    def test_old_viewer_is_rejected_before_a_material_snapshot_is_sent(self):
        received = []
        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(3)
                self.assertEqual(receive(connection, 8), module.MAGIC)
                connection.sendall(b'{"protocol":1,"joints":["mPelvis"]}\n')
                received.append(connection.recv(1))
        client, thread, failures = self.serve(server)
        client.submit({"revision":1})
        self.finish(client, thread, failures)
        self.assertEqual(received, [b""], "an older viewer must receive no unsupported material data")

    def serve(self, callback):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(6)
        failures = []

        def worker():
            try:
                callback(listener)
            except Exception as error:
                failures.append(error)
            finally:
                listener.close()

        thread = threading.Thread(target=worker, daemon=True)
        thread.start()
        client = module.Client(listener.getsockname()[1])
        self.addCleanup(client.close)
        return client, thread, failures

    def finish(self, client, thread, failures):
        thread.join(6)
        client.close()
        client._thread.join(2)
        self.assertFalse(thread.is_alive())
        self.assertFalse(client._thread.is_alive())
        self.assertEqual(failures, [])

    def test_latest_snapshot_replaces_queued_edits(self):
        release = threading.Event()
        received = []

        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(5)
                release.wait(3)
                hello(connection)
                received.append(snapshot(connection)["revision"])
                connection.sendall(b'{"ok":true}\n')

        client, thread, failures = self.serve(server)
        for revision in range(1, 21):
            client.submit({"protocol":1,"revision":revision,"meshes":[]})
        release.set()
        self.finish(client, thread, failures)
        self.assertEqual(received, [20])

    def test_reconnect_resends_latest_without_another_edit(self):
        received = []

        def server(listener):
            for _ in range(2):
                with listener.accept()[0] as connection:
                    connection.settimeout(5)
                    hello(connection)
                    received.append(snapshot(connection)["revision"])
                    connection.sendall(b'{"ok":true}\n')

        client, thread, failures = self.serve(server)
        client.submit({"protocol":1,"revision":7,"meshes":[]})
        self.finish(client, thread, failures)
        self.assertEqual(received, [7,7])

    def test_rejected_snapshot_can_be_corrected_on_same_connection(self):
        rejected = threading.Event()
        corrected = threading.Event()

        def server(listener):
            with listener.accept()[0] as connection:
                connection.settimeout(5)
                hello(connection)
                self.assertEqual(snapshot(connection)["revision"], 1)
                connection.sendall(b'{"ok":false,"error":"Invalid weight"}\n')
                rejected.set()
                self.assertEqual(snapshot(connection)["revision"], 2)
                connection.sendall(b'{"ok":true}\n')
                corrected.set()

        client, thread, failures = self.serve(server)
        client.submit({"protocol":1,"revision":1,"meshes":[]})
        self.assertTrue(rejected.wait(3))
        deadline = time.monotonic() + 2
        while "Invalid weight" not in client.state()[0] and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertIn("Invalid weight", client.state()[0])
        client.submit({"protocol":1,"revision":2,"meshes":[]})
        self.assertTrue(corrected.wait(3))
        self.finish(client, thread, failures)

    def test_response_line_is_bounded_and_requires_a_newline(self):
        for value in (b"", b"{}", b"x" * (module.MAX_REPLY + 1) + b"\n", b"garbage\n"):
            with self.assertRaises(ValueError):
                module.Client._line(io.BytesIO(value))


if __name__ == "__main__":
    unittest.main()
