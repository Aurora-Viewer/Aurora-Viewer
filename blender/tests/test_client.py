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
    connection.sendall(b'{"protocol":1,"joints":["mPelvis"],"max_bytes":67108864}\n')


def snapshot(connection):
    length = struct.unpack("<I", receive(connection, 4))[0]
    return json.loads(receive(connection, length))


class ClientTests(unittest.TestCase):
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
