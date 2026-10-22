"""Loopback-only transport. This worker never imports or accesses bpy."""

import json
import socket
import struct
import threading
import time

MAGIC = b"AURBLD1\n"
MAX_BYTES = 64 * 1024 * 1024
MAX_REPLY = 256 * 1024


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
        self._status = "Connecting to Aurora…"
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
        return json.loads(line)

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
                        self._set_status("Connected to Aurora", hello["joints"])
                        sent = None
                        heartbeat = time.monotonic()
                        while not self._stop.is_set():
                            self._wake.wait(0.1)
                            self._wake.clear()
                            with self._lock:
                                latest = self._latest
                            if latest is not None and latest["revision"] != sent:
                                payload = json.dumps(latest, separators=(",", ":"), allow_nan=False).encode("utf-8")
                                if len(payload) > min(MAX_BYTES, hello.get("max_bytes", MAX_BYTES)):
                                    self._set_status("Mesh snapshot exceeds the 64 MiB limit")
                                    sent = latest["revision"]
                                    continue
                                connection.sendall(struct.pack("<I", len(payload)) + payload)
                                reply = self._line(reader)
                                sent = latest["revision"]
                                if reply.get("ok"):
                                    self._set_status("Connected · revision {}".format(sent))
                                else:
                                    self._set_status("Aurora: " + str(reply.get("error", "Snapshot rejected")))
                                    if reply.get("retry"):
                                        sent = None
                                        self._stop.wait(0.5)
                                heartbeat = time.monotonic()
                            elif time.monotonic() - heartbeat > 2.0:
                                connection.sendall(b"\0\0\0\0")
                                self._line(reader)
                                heartbeat = time.monotonic()
            except (OSError, ValueError, TypeError) as error:
                self._set_status("Waiting for Aurora: " + str(error), ())
                self._stop.wait(1.0)
            finally:
                with self._lock:
                    self._connection = None
