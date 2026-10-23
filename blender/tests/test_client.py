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
