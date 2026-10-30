# /// script
# requires-python = ">=3.10,<3.11"
# dependencies = ["lummao==0.5.2"]
# ///
"""Exercise the actual bridge scripts offline: uv run scripts/test-bridge-lsl.py.

Lummao compiles the complete LSL sources; only simulator APIs are mocked.
This tests event ordering and emitted traffic, not simulator physics or Mono's
compiled memory budget. No network connection is made by the test harness.
"""

import unittest
from pathlib import Path

import lummao


SOURCE = Path(__file__).resolve().parents[1] / "crates/aurora-viewer/src/world/bridge"
OWNER = lummao.Key("00000000-0000-0000-0000-000000000001")
ROOT = lummao.Key("00000000-0000-0000-0000-000000000002")
OTHER = lummao.Key("00000000-0000-0000-0000-000000000003")
NULL = lummao.Key("00000000-0000-0000-0000-000000000000")
DISCOVERY = -184735219
REGION = 256
REGION_START = 1024
TELEPORT = 512
PERMISSIONS = 4 | 32 | 1024
SCRIPTS = ("bridge.lsl", "query.lsl", "ik.lsl")


class ResetScript(Exception):
    pass


class Simulator:
    def __init__(self, filename):
        self.filename = filename
        self.now = 0.0
        self.attached = 31
        self.permissions = PERMISSIONS
        self.permission_requests = []
        self.messages = []
        self.responses = []
        self.broadcasts = []
        self.detaches = 0
        self.timer = 0.0
        self.serial = 100
        self.requests = []
        self.listeners = {}
        self.prim_updates = []
        self.targets = []
        self.reset()

    def reset(self):
        self.listeners.clear()
        self.script = lummao.compile_script_file(SOURCE / self.filename)
        builtins = self.script.builtin_funcs
        functions = {
            "llGetAttached": lambda: self.attached,
            "llGetOwner": lambda: OWNER,
            "llGetKey": lambda: ROOT,
            "llGetTime": lambda: self.now,
            "llGetPermissions": lambda: self.permissions,
            "llGetPermissionsKey": lambda: OWNER,
            "llRequestPermissions": self.request_permissions,
            "llRequestSecureURL": self.request_url,
            "llGenerateKey": self.key,
            "llListen": self.listen,
            "llListenRemove": lambda handle: self.listeners.pop(handle, None),
            "llOwnerSay": self.messages.append,
            "llHTTPResponse": lambda *args: self.responses.append(args),
            "llRegionSay": lambda *args: self.broadcasts.append(args),
            "llRegionSayTo": lambda *args: self.broadcasts.append(args),
            "llGetPos": lambda: lummao.Vector((128.0, 128.0, 25.0)),
            "llGetVel": lambda: lummao.Vector((0.0, 0.0, 0.0)),
            "llGetAgentInfo": lambda *args: 0,
            "llGetAgentSize": lambda *args: lummao.Vector((0.0, 0.0, 0.0)),
            "llGetOwnerKey": lambda *args: OTHER,
            "llGetObjectDetails": lambda *args: [31],
            "llGetFreeMemory": lambda: 32768,
            "llGetMinMemory": lambda: 32768,
            "llSetTimerEvent": self.set_timer,
            "llDetachFromAvatar": self.detach,
            "llResetScript": self.reset_request,
            "llSetLinkPrimitiveParamsFast": lambda link, rules: self.prim_updates.append((link, rules)),
            "llMoveToTarget": lambda target, tau: self.targets.append((target, tau)),
        }
        for name in (
            "llReleaseURL", "llTakeControls", "llReleaseControls",
            "llSetForce", "llStopMoveToTarget", "llSetVehicleType",
            "llSetVehicleFloatParam", "llTargetRemove",
        ):
            functions[name] = lambda *args: None
        for name, function in functions.items():
            builtins[name] = function

    def key(self):
        self.serial += 1
        return lummao.Key(f"00000000-0000-0000-0000-{self.serial:012d}")

    def request_url(self):
        request = self.key()
        self.requests.append(request)
        return request

    def listen(self, *arguments):
        self.serial += 1
        self.listeners[self.serial] = arguments
        return self.serial

    def request_permissions(self, owner, permissions):
        assert owner == OWNER
        self.permission_requests.append(permissions)
        self.permissions = permissions
        self.script.queue_event("run_time_permissions", (permissions,))

    def set_timer(self, seconds):
        self.timer = seconds

    def detach(self):
        assert self.attached and self.permissions & 32, "unsafe detach would emit a simulator error"
        self.detaches += 1
        self.attached = 0
        self.script.queue_event("attach", (NULL,))

    @staticmethod
    def reset_request():
        raise ResetScript()

    async def drain(self):
        try:
            await self.script.execute()
        except ResetScript:
            self.reset()
            await self.script.execute()

    async def event(self, name, *arguments):
        self.script.queue_event(name, arguments)
        await self.drain()

    async def start(self):
        await self.drain()
        await self.event("on_rez", 0)
        await self.event("attach", OWNER)

    async def grant(self, method="URL_REQUEST_GRANTED"):
        if self.requests:
            await self.event("http_request", self.requests[-1], method, "https://sim.example.invalid/cap/test")

    async def probe(self, owner=OWNER, channel=DISCOVERY, message=None):
        await self.event("listen", channel, "", owner, message or f"auroraHello|{ROOT}")


class BridgeScripts(unittest.IsolatedAsyncioTestCase):
    async def test_container_is_an_invisible_default_cube_on_start_and_reattachment(self):
        sim = Simulator("bridge.lsl")
        for _ in range(2):
            sim.prim_updates.clear()
            await sim.start()
            self.assertTrue(sim.prim_updates, "normalize even an existing rock container")
            link, rules = sim.prim_updates[-1]
            self.assertEqual(link, -4)  # LINK_THIS
            parameters = {}
            # LSL primitive parameter arities, independent of rule order.
            arities = {9: 7, 7: 1, 17: 5, 18: 3, 28: 1}
            while rules:
                flag = rules[0]
                count = arities[flag]
                parameters[flag] = rules[1:count + 1]
                rules = rules[count + 1:]
            vector = lummao.Vector
            self.assertEqual(parameters[9], [0, 0, vector((0, 1, 0)), 0.0,
                                          vector((0, 0, 0)), vector((1, 1, 0)), vector((0, 0, 0))])
            self.assertEqual(parameters[7], [vector((0.01, 0.01, 0.01))])
            self.assertEqual(parameters[17], [-1, "89556747-24cb-43ed-920b-47caed15465f",
                                            vector((1, 1, 0)), vector((0, 0, 0)), 0.0])
            self.assertEqual(parameters[18], [-1, vector((1, 1, 1)), 0.0])
            self.assertEqual(parameters[28], [""])
            await sim.event("attach", NULL)

    async def test_cold_start_and_url_grants_are_silent_without_aurora(self):
        for filename in SCRIPTS:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                await sim.grant()
                await sim.event("timer")
                self.assertEqual(sim.messages, [])

    async def test_url_denials_and_retries_are_silent_without_aurora(self):
        for filename in SCRIPTS[:2]:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                for now in (5.0, 10.0, 15.0):
                    sim.now = now
                    await sim.grant("URL_REQUEST_DENIED")
                    await sim.event("timer")
                self.assertEqual(sim.messages, [])

    async def test_only_the_owner_addressing_this_root_can_enable_output(self):
        for filename in SCRIPTS:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                await sim.grant()
                await sim.probe(owner=OTHER)
                await sim.probe(channel=DISCOVERY - 1)
                await sim.probe(message=f"auroraHello|{OTHER}")
                self.assertEqual(sim.messages, [])
                await sim.probe()
                self.assertEqual(len(sim.messages), 1)
                self.assertIn("1.40|", sim.messages[0])
                await sim.probe()
                self.assertEqual(len(sim.messages), 1, "duplicate probes must be throttled")

    async def test_a_probe_before_url_or_permissions_is_fulfilled_when_ready(self):
        for filename in SCRIPTS[:2]:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                if filename == "bridge.lsl":
                    sim.permissions = 0
                    sim.script.gPermissionsReady = 0
                await sim.probe()
                await sim.grant()
                if filename == "bridge.lsl":
                    self.assertFalse(any("<auroraBridge>" in text for text in sim.messages))
                    sim.permissions = PERMISSIONS
                    await sim.event("run_time_permissions", PERMISSIONS)
                self.assertTrue(any("<auroraBridge>" in text or "<auroraBridgeQuery>" in text for text in sim.messages))

    async def test_region_changes_stay_silent_on_an_unsupported_viewer(self):
        for filename in SCRIPTS:
            for change in (REGION, REGION_START, TELEPORT):
                with self.subTest(script=filename, change=change):
                    sim = Simulator(filename)
                    await sim.start()
                    await sim.grant()
                    await sim.event("changed", change)
                    await sim.grant()
                    self.assertEqual(sim.messages, [])

    async def test_confirmed_aurora_survives_duplicate_region_events_without_a_probe(self):
        for filename in SCRIPTS:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                await sim.grant()
                await sim.probe()
                for change in (REGION_START, REGION, REGION_START):
                    previous_session = getattr(sim.script, "session", None)
                    sim.messages.clear()
                    await sim.event("changed", change)
                    await sim.grant()
                    self.assertEqual(len(sim.messages), 1, "the current Aurora login must be reconnected immediately")
                    if filename == "ik.lsl":
                        self.assertNotEqual(sim.script.session, previous_session)
                        self.assertEqual(len(sim.listeners), 3, "retired listeners must not accumulate")

    async def test_discovery_still_works_after_unpaired_crossings(self):
        for filename in SCRIPTS:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                for now in (10.0, 20.0):
                    sim.now = now
                    await sim.event("changed", REGION)
                    await sim.grant()
                    await sim.probe()
                    self.assertTrue(sim.messages)
                    await sim.event("on_rez", 0)
                    await sim.event("attach", OWNER)
                    sim.messages.clear()

    async def test_login_revokes_old_pairing_before_delayed_callbacks(self):
        for filename in SCRIPTS:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                await sim.grant()
                await sim.probe()
                sim.messages.clear()
                await sim.event("on_rez", 0)
                await sim.grant()
                await sim.event("attach", OWNER)
                await sim.grant()
                self.assertEqual(sim.messages, [])
                for now in (10.0, 20.0, 60.0):
                    sim.now = now
                    await sim.event("changed", REGION)
                    await sim.grant()
                    await sim.event("timer")
                    self.assertEqual(sim.messages, [], "switching viewers must stay silent through further crossings")
                if filename == "bridge.lsl":
                    self.assertEqual(sim.detaches, 1)

    async def test_detaches_after_sixty_seconds_without_any_chat(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        await sim.grant()
        sim.now = 59.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 0)
        sim.now = 60.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 1)
        self.assertEqual(sim.messages, [])
        self.assertTrue(all(permissions & 32 for permissions in sim.permission_requests))

    async def test_repeated_crossings_cannot_postpone_unsupported_detachment(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        for now in (10.0, 20.0, 59.0):
            sim.now = now
            await sim.event("changed", REGION)
            await sim.grant()
        sim.now = 60.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 1)
        self.assertEqual(sim.messages, [])

    async def test_a_late_valid_probe_cancels_detachment_without_idle_polling(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        await sim.grant()
        sim.now = 59.0
        await sim.probe()
        self.assertEqual(sim.timer, 0.0)
        sim.now = 600.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 0)
        await sim.event("on_rez", 0)
        await sim.event("attach", OWNER)
        sim.messages.clear()
        sim.now = 659.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 0)
        sim.now = 660.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 1)
        self.assertEqual(sim.messages, [])

    async def test_missing_attach_permission_is_requested_without_an_unsafe_detach(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        sim.permissions = 0
        sim.now = 60.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 0)
        sim.now = 65.0
        await sim.event("timer")
        self.assertEqual(sim.detaches, 1)
        self.assertEqual(sim.messages, [])

    async def test_old_http_credentials_cannot_operate_an_unpaired_script(self):
        for filename in SCRIPTS[:2]:
            with self.subTest(script=filename):
                sim = Simulator(filename)
                await sim.start()
                await sim.event("http_request", OTHER, "POST", "old-session-request")
                self.assertEqual(sim.responses[-1][1], 403)
                self.assertEqual(sim.messages, [])
                self.assertEqual(sim.broadcasts, [])

    async def test_current_aurora_commands_still_work_after_discovery(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        await sim.grant()
        await sim.probe()
        await sim.event("http_request", OTHER, "POST", f"lock|{sim.script.gToken}|1|1|1|0|0|0|0|0")
        self.assertEqual(sim.responses[-1][1], 200)
        self.assertEqual(sim.script.gEnabled, 1)
        self.assertEqual(sim.script.gLocked, 1)
        # As movelockMe: the anchor sits 10 cm below the avatar so the floor,
        # not a gravity balance, holds it (#1248), and later ticks keep it.
        await sim.event("timer")
        await sim.event("timer")
        self.assertTrue(sim.targets)
        for target, tau in sim.targets:
            self.assertAlmostEqual(target[2], 24.9, places=4)
            self.assertEqual((target[0], target[1], tau), (128.0, 128.0, 0.05))

        sim = Simulator("query.lsl")
        await sim.start()
        await sim.grant()
        await sim.probe()
        await sim.event("http_request", OTHER, "POST", f"query|{sim.script.gToken}|pose|request,{OTHER},{OTHER}")
        self.assertEqual(sim.responses[-1][1], 200)
        self.assertEqual(sim.broadcasts[-1][0], OTHER)

        sim = Simulator("ik.lsl")
        await sim.start()
        await sim.probe()
        await sim.event("listen", -190018106, "", OWNER, f"auroraIKRelay|{sim.script.session}|discover")
        self.assertEqual(len(sim.broadcasts), 1)
        await sim.event("listen", -190018107, "", OTHER, "auroraIKAvailable|test")
        self.assertTrue(sim.messages[-1].startswith("<auroraIKRelay>"))

    async def test_unpaired_region_services_never_forward_peer_traffic(self):
        sim = Simulator("query.lsl")
        await sim.start()
        await sim.event("listen", -184735221, "", OTHER, "portal2|test")
        await sim.event("listen", -184735220, "", OTHER, f"pose1|{OTHER}|request|{OWNER}")
        self.assertEqual(sim.messages, [])
        sim = Simulator("ik.lsl")
        await sim.start()
        await sim.event("listen", -190018106, "", OWNER, f"auroraIKRelay|{sim.script.session}|discover")
        await sim.event("listen", -190018107, "", OTHER, "auroraIKAvailable|test")
        self.assertEqual(sim.messages, [])
        self.assertEqual(sim.broadcasts, [])

    async def test_revoking_pairing_suppresses_the_final_motor_report(self):
        sim = Simulator("bridge.lsl")
        await sim.start()
        await sim.grant()
        await sim.probe()
        sim.script.gEnabled = 1
        sim.script.gMode = 5
        sim.script.gPassFinished = 0
        sim.script.gSequence = 1
        sim.messages.clear()
        await sim.event("on_rez", 0)
        await sim.event("attach", OWNER)
        self.assertEqual(sim.messages, [])
        self.assertEqual(sim.script.gMode, 0)
        self.assertEqual(sim.script.gEnabled, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
