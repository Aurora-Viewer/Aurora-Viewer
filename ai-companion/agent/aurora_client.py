"""Local TCP client for the Aurora native AI bridge; no credentials or LSL."""
from __future__ import annotations
import json
import os
from pathlib import Path
import socket
import tempfile

HOST = "127.0.0.1"
PORT = 57833


class AuroraError(RuntimeError):
    pass


class AuroraViewer:
    def __init__(self, token: str | None = None):
        self.token = token or os.environ.get("AURORA_AI_TOKEN", "")
        if len(self.token) < 32:
            raise AuroraError("AURORA_AI_TOKEN is missing or too short. Launch with start_aurora_ai.ps1")

    def request(self, op: str, **params):
        payload = json.dumps({"token": self.token, "op": op, **params}).encode() + b"\n"
        if len(payload) > 8192:
            raise AuroraError("Request exceeds viewer bridge limit")
        with socket.create_connection((HOST, PORT), timeout=6) as sock:
            sock.settimeout(6)
            sock.sendall(payload)
            sock.shutdown(socket.SHUT_WR)
            result = bytearray()
            while b"\n" not in result:
                chunk = sock.recv(4096)
                if not chunk:
                    raise AuroraError("Viewer disconnected without replying")
                result.extend(chunk)
                if len(result) > 64_000:
                    raise AuroraError("Oversized viewer reply")
        value = json.loads(result.split(b"\n", 1)[0])
        if not value.get("ok"):
            raise AuroraError(str(value.get("error", "unknown viewer error")))
        return value

    def observe(self, screenshot=False):
        value = self.request("observe")
        value["screenshot"] = capture_window() if screenshot else None
        return value

    def move(self, direction, duration_ms):
        return self.request("move", direction=direction, duration_ms=duration_ms)

    def chat(self, text):
        return self.request("chat", text=text)

    def close(self):
        try:
            self.request("stop")
        except (OSError, AuroraError):
            pass


def capture_window():
    """Capture *only* this launch's Aurora window. Requires Windows + mss."""
    if os.name != "nt":
        return None
    try:
        import ctypes
        from ctypes import wintypes
        import mss
        from PIL import Image
        pid = int(os.environ.get("AURORA_AI_VIEWER_PID", "0"))
        if not pid:
            return None  # Never capture an arbitrary app by title alone.
        u32 = ctypes.windll.user32
        u32.IsWindowVisible.argtypes = [wintypes.HWND]
        u32.IsWindowVisible.restype = wintypes.BOOL
        u32.IsIconic.argtypes = [wintypes.HWND]
        u32.IsIconic.restype = wintypes.BOOL
        u32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        u32.GetWindowThreadProcessId.restype = wintypes.DWORD
        u32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        u32.GetWindowTextW.restype = ctypes.c_int
        u32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        u32.GetClientRect.restype = wintypes.BOOL
        u32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
        u32.ClientToScreen.restype = wintypes.BOOL
        handles = []
        callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)

        def find_window(hwnd, _):
            if u32.IsWindowVisible(hwnd) == 0 or u32.IsIconic(hwnd):
                return True
            process = wintypes.DWORD()
            u32.GetWindowThreadProcessId(hwnd, ctypes.byref(process))
            if process.value != pid:
                return True
            title = ctypes.create_unicode_buffer(512)
            u32.GetWindowTextW(hwnd, title, 512)
            if "aurora" in title.value.lower():
                handles.append(hwnd)
            return True

        cb = callback_type(find_window)
        u32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
        u32.EnumWindows.restype = wintypes.BOOL
        u32.EnumWindows(cb, 0)
        if not handles:
            return None
        rect = wintypes.RECT()
        u32.GetClientRect(handles[0], ctypes.byref(rect))
        origin = wintypes.POINT(0, 0)
        u32.ClientToScreen(handles[0], ctypes.byref(origin))
        width, height = rect.right - rect.left, rect.bottom - rect.top
        if width < 200 or height < 150:
            return None
        # A small image is considerably cheaper for an RTX 3070 vision model.
        with mss.mss() as sct:
            image = sct.grab({"left": origin.x, "top": origin.y, "width": width, "height": height})
            photo = Image.frombytes("RGB", image.size, image.rgb)
            photo.thumbnail((1280, 720))
            file = tempfile.NamedTemporaryFile(prefix="aurora_ai_", suffix=".jpg", delete=False)
            file.close()
            photo.save(file.name, "JPEG", quality=80)
            return file.name
    except Exception:  # Camera is optional: fall back to structured world state.
        return None
