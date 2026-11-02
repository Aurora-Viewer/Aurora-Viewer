import json
import socket
from unittest import TestCase
from unittest.mock import patch
from aurora_agent import new_addressed_message, safe_action
from aurora_client import AuroraViewer, AuroraError

class FakeViewer:
    def __init__(self):
        self.calls = []
    def move(self, direction, duration_ms):
        self.calls.append(("move", direction, duration_ms))
        return {"ok":True}
    def chat(self, text):
        self.calls.append(("chat", text))

class Safeguards(TestCase):
    def test_move_not_enabled_or_out_of_bounds(self):
        viewer = FakeViewer()
        proposal = {"action":"move","direction":"forward","duration_ms":500}
        self.assertEqual(safe_action(proposal, viewer, {"allow_walking":False}, None)[0], "wait")
        self.assertEqual(safe_action({**proposal,"duration_ms":2000}, viewer, {"allow_walking":True}, None)[0], "wait")
        self.assertEqual(viewer.calls, [])
        self.assertEqual(safe_action(proposal, viewer, {"allow_walking":True}, None)[0], "move")
        self.assertEqual(viewer.calls[-1], ("move","forward",500))

    def test_chat_only_when_addressed_and_enabled(self):
        viewer = FakeViewer()
        reply = {"action":"reply","reply_text":"Hello there!"}
        self.assertEqual(safe_action(reply, viewer, {"allow_chat":True}, None)[0],"wait")
        self.assertEqual(safe_action(reply, viewer, {"allow_chat":False}, {"text":"Hi Mira"})[0],"wait")
        self.assertEqual(safe_action(reply, viewer, {"allow_chat":True}, {"text":"Hi Mira"})[0],"reply")
        self.assertEqual(viewer.calls, [("chat","Hello there!")])

    def test_chat_message_new_once(self):
        obs = {"nearby_chat":[{"source":"uuid","time_ms":1234,"text":"Mira, hi","from":"Friend"}]}
        seen = set()
        self.assertEqual(new_addressed_message(obs,"Mira",seen)["from"],"Friend")
        self.assertIsNone(new_addressed_message(obs,"Mira",seen))

    def test_json_token_present(self):
        v = AuroraViewer("a"*40)
        class Socket:
            def __enter__(self): return self
            def __exit__(self, *args): pass
            def settimeout(self, n): pass
            def sendall(self, data):
                raw = json.loads(data)
                assert raw["token"] == "a"*40
                assert raw["op"] == "status"
            def shutdown(self, how): pass
            def recv(self, n): return b'{"ok":true,"logged_in":true}\n'
        with patch("aurora_client.socket.create_connection", return_value=Socket()):
            self.assertTrue(v.request("status")["logged_in"])

    def test_no_token_fails(self):
        with self.assertRaises(AuroraError):
            AuroraViewer("short")
