import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from companion import Memory, DemoViewer, perform, DEFAULT_CONFIG, scripted_decision

class CompanionTests(unittest.TestCase):
    def test_memory_survives_reopening(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / 'mem.db'
            store = Memory(p)
            store.journal('I saw a simulated garden')
            store.conn.close()
            reopened = Memory(p)
            self.assertIn('garden', reopened.history()[0]['entry'])
            self.assertEqual(len(reopened.summary()['goals']), 3)
            reopened.conn.close()

    def test_movement_protected(self):
        cfg = DEFAULT_CONFIG.copy()
        v = DemoViewer()
        before = v.pos[:]
        action, result = perform({'action':'wander', 'dx':9, 'dy':1}, v, cfg, True)
        self.assertEqual(action, 'wait')
        self.assertEqual(v.pos, before)
        action, result = perform({'action':'wander', 'dx':2, 'dy':2}, v, cfg, False)
        self.assertEqual(action, 'wait')
        self.assertEqual(v.pos, before)

    def test_scripted_decisions_well_formed(self):
        for n in range(9):
            act, result = perform(scripted_decision(n), DemoViewer(), DEFAULT_CONFIG, True)
            self.assertIn(act, ('wait','journal','wander'))

class OllamaPayloadTests(unittest.TestCase):
    def test_ollama_request_and_json_decision(self):
        from companion import decide_ollama
        from unittest.mock import patch
        from io import BytesIO
        class FakeResponse(BytesIO):
            def __enter__(self): return self
            def __exit__(self, *args): self.close()
        captured = {}
        def fake_urlopen(req, timeout):
            captured['payload'] = json.loads(req.data)
            payload = {'message': {'content': json.dumps({'action': 'journal', 'reason': 'reflect', 'reflection': 'Quiet simulated garden', 'dx': 0, 'dy': 0})}}
            return FakeResponse(json.dumps(payload).encode())
        with patch('companion.urlopen', fake_urlopen):
            result = decide_ollama(DEFAULT_CONFIG, {'text':'Explore'}, {'logged_in':True, 'scene_hint':'Garden','screenshot':None}, [], 'Kind and curious')
        self.assertEqual(result['action'], 'journal')
        self.assertEqual(captured['payload']['model'], 'qwen3-vl:4b-instruct')
        self.assertEqual(captured['payload']['messages'][1]['role'], 'user')

class LeapFramingTests(unittest.TestCase):
    def test_length_framing(self):
        from companion import LeapViewer
        from io import BytesIO
        import types
        test = LeapViewer.__new__(LeapViewer)
        test.llsd = types.SimpleNamespace(format_notation=lambda obj: json.dumps(obj).encode(), parse=lambda raw: json.loads(raw))
        test.reader = BytesIO()
        test.writer = BytesIO()
        test.send('my_pump', {'op':'test'})
        frame = test.writer.getvalue()
        count, raw = frame.split(b':', 1)
        self.assertEqual(len(raw), int(count))
        test.reader = BytesIO(frame)
        self.assertEqual(test._read(), {'pump':'my_pump', 'data':{'op':'test'}})

class GoalProposalTests(unittest.TestCase):
    def test_model_goal_suggestions_are_bounded(self):
        with tempfile.TemporaryDirectory() as tmp:
            store = Memory(Path(tmp) / 'goals.db')
            self.assertTrue(store.propose_goal('Notice the shapes of nearby trees'))
            self.assertFalse(store.propose_goal('Make purchases in shops'))
            self.assertFalse(store.propose_goal('Notice the shapes of nearby trees'))
            store.conn.close()

if __name__ == '__main__':
    unittest.main()
