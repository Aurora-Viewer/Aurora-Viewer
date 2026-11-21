# /// script
# requires-python = ">=3.10,<3.11"
# dependencies = ["lummao==0.5.2"]
# ///
"""Exercise the actual bridge scripts offline: uv run scripts/test-bridge-lsl.py.

Lummao compiles the complete LSL sources; only simulator APIs are mocked.
This tests event ordering and emitted traffic, not simulator physics or Mono's
compiled memory budget. No network connection is made by the test harness.
"""

import base64
import unittest
from pathlib import Path

import lummao


SOURCE = Path(__file__).resolve().parents[1] / "crates/aurora-viewer/src/world/bridge"
OWNER = lummao.Key("00000000-0000-0000-0000-000000000001")
# Hex letters in the first digits exercise the channel derivation.
ROOT = lummao.Key("1a2b3c4d-0000-0000-0000-000000000002")
OTHER = lummao.Key("00000000-0000-0000-0000-000000000003")
NULL = lummao.Key("00000000-0000-0000-0000-000000000000")
SEAT = lummao.Key("00000000-0000-0000-0000-000000000004")


def private_channel(key):
    """world/bridge.rs private_channel: the first seven hex digits of the key."""
    return -0x10000000 - int(str(key)[:7], 16)


DISCOVERY = private_channel(ROOT)
# query.lsl's consent dialog channel, and the stop channel residents type on
# (aurora_net::bridge::query::POSE_STOP_CHANNEL).
POSE_DIALOG = -0x40000000 - int(str(ROOT)[:7], 16)
STOP_CHANNEL = 2525
LEGACY_DISCOVERY = -184735219
OBJECT_ROOT = 18
REGION = 256
REGION_START = 1024
TELEPORT = 512
PERMISSIONS = 4 | 32 | 1024
SCRIPTS = ("bridge.lsl", "query.lsl")


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
        self.dialogs = []
        self.ims = []
        self.regionwide = []
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
            "llRegionSay": self.region_say,
            "llRegionSayTo": lambda *args: self.broadcasts.append(args),
            "llDialog": lambda *args: self.dialogs.append(args),
            "llInstantMessage": lambda *args: self.ims.append(args),
            "llGetPos": lambda: lummao.Vector((128.0, 128.0, 25.0)),
            "llGetVel": lambda: lummao.Vector((0.0, 0.0, 0.0)),
            "llGetAgentInfo": lambda *args: 0,
            "llGetAgentSize": lambda *args: lummao.Vector((0.0, 0.0, 0.0)),
            "llGetOwnerKey": lambda *args: OTHER,
            "llGetObjectDetails": self.object_details,
            "llGetAgentList": lambda *args: [OWNER, OTHER],
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
        return lummao.Key(f"{self.serial:08d}-0000-0000-0000-{self.serial:012d}")

    def region_say(self, *args):
        self.regionwide.append(args)
        self.broadcasts.append(args)

    @staticmethod
    def object_details(target, details):
        # OTHER sits on SEAT; the wearer stands. Attachment points otherwise.
        if details == [OBJECT_ROOT]:
            return [SEAT if target == OTHER else target]
        return [31]

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

    def listening(self, channel):
        return any(arguments[0] == channel for arguments in self.listeners.values())

    async def ask(self, nonce, target=OTHER, texts=None):
        texts = texts or ("May I pose you?", "Yes", "No", "Type /2525 stop.", "Stop", "Stopped.")
        encoded = ",".join(base64.b64encode(t.encode()).decode() for t in texts)
        await self.event("http_request", OTHER, "POST", f"query|{self.script.gToken}|pose|ask,{nonce},{target},{encoded}")


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
                await sim.probe(channel=LEGACY_DISCOVERY)
                await sim.probe(message=f"auroraHello|{OTHER}")
                self.assertEqual(sim.messages, [])
                await sim.probe()
                self.assertEqual(len(sim.messages), 1)
                self.assertIn("1.44|", sim.messages[0])
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
                    sim.messages.clear()
                    await sim.event("changed", change)
                    await sim.grant()
                    self.assertEqual(len(sim.messages), 1, "the current Aurora login must be reconnected immediately")

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
        await sim.ask(SEAT)
        self.assertEqual(sim.responses[-1][1], 200)
        self.assertEqual(sim.dialogs[-1][0], OTHER)

    async def test_unpaired_region_services_never_forward_peer_traffic(self):
        sim = Simulator("query.lsl")
        await sim.start()
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Yes")
        await sim.event("listen", STOP_CHANNEL, "", OTHER, "stop")
        self.assertEqual(sim.messages, [])

    async def test_the_query_script_relays_no_portal(self):
        # 1.43: portals are the viewer's own; nothing listens on or speaks
        # over the old portal channel.
        sim = Simulator("query.lsl")
        await sim.start()
        await sim.grant()
        await sim.probe()
        sim.messages.clear()
        self.assertNotIn(-184735221, [arguments[0] for arguments in sim.listeners.values()])
        await sim.event("listen", -184735221, "", OTHER, "portal2|test")
        self.assertEqual(sim.messages, [])
        token = sim.script.gToken
        await sim.event("http_request", OTHER, "POST", f"query|{token}|portal|{NULL},e30=")
        self.assertEqual(sim.responses[-1][1], 400)
        self.assertEqual(sim.broadcasts, [])

    async def test_pose_consent_is_a_dialog_the_resident_answers(self):
        sim = Simulator("query.lsl")
        await sim.start()
        await sim.grant()
        await sim.probe()
        first = lummao.Key("00000000-0000-0000-0000-0000000000a1")
        await sim.ask(first, texts=("Puis-je ?", "Oui", "Non", "/2525 stop", "Arrêter", "Pose arrêtée."))
        self.assertEqual(sim.responses[-1][1], 200)
        # a standard dialog to the resident, in the poser's language, and
        # nothing said to any other viewer
        self.assertEqual(sim.dialogs, [(OTHER, "Puis-je ?", ["Oui", "Non"], POSE_DIALOG)])
        self.assertEqual(sim.broadcasts, [])
        self.assertTrue(sim.listening(POSE_DIALOG))
        self.assertFalse(sim.listening(STOP_CHANNEL), "no stop listener before anyone accepts")
        # someone else's answer, or another word, is not theirs
        count = len(sim.messages)
        await sim.event("listen", POSE_DIALOG, "", SEAT, "Oui")
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Peut-être")
        self.assertEqual(len(sim.messages), count)
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Oui")
        self.assertEqual(sim.messages[-1], f"<auroraPose>{first}|accept|{OTHER}</auroraPose>")
        # how to stop, with a Stop button for when chat is out of range
        self.assertEqual(sim.dialogs[-1], (OTHER, "/2525 stop", ["Arrêter"], POSE_DIALOG))
        # and the same in an IM, which stays in their history
        self.assertEqual(sim.ims, [(OTHER, "/2525 stop")])
        self.assertTrue(sim.listening(STOP_CHANNEL))
        await sim.event("listen", STOP_CHANNEL, "", SEAT, "stop")
        self.assertNotIn("withdraw", sim.messages[-1])
        await sim.event("listen", STOP_CHANNEL, "", OTHER, "  STOP ")
        self.assertEqual(sim.messages[-1], f"<auroraPose>{first}|withdraw|{OTHER}</auroraPose>")
        self.assertEqual(sim.broadcasts[-1], (OTHER, 0, "Pose arrêtée."))
        self.assertFalse(sim.listening(STOP_CHANNEL) or sim.listening(POSE_DIALOG), "nobody left to hear")
        # the Stop button ends it too
        second = lummao.Key("00000000-0000-0000-0000-0000000000a2")
        await sim.ask(second)
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Yes")
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Stop")
        self.assertEqual(sim.messages[-1], f"<auroraPose>{second}|withdraw|{OTHER}</auroraPose>")
        # No is a refusal
        third = lummao.Key("00000000-0000-0000-0000-0000000000a3")
        await sim.ask(third)
        await sim.event("listen", POSE_DIALOG, "", OTHER, "No")
        self.assertEqual(sim.messages[-1], f"<auroraPose>{third}|refuse|{OTHER}</auroraPose>")
        self.assertEqual(len(sim.ims), 2, "an IM only for each Yes")
        # forgotten by the viewer (timed out, cancelled): a late Yes is ignored
        fourth = lummao.Key("00000000-0000-0000-0000-0000000000a4")
        await sim.ask(fourth)
        await sim.event("http_request", OTHER, "POST", f"query|{sim.script.gToken}|pose|forget,{fourth},{OTHER}")
        self.assertEqual(sim.responses[-1][1], 200)
        count = len(sim.messages)
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Yes")
        self.assertEqual(len(sim.messages), count)
        # a region change ends every consent
        fifth = lummao.Key("00000000-0000-0000-0000-0000000000a5")
        await sim.ask(fifth)
        await sim.event("listen", POSE_DIALOG, "", OTHER, "Yes")
        await sim.event("changed", REGION)
        count = len(sim.messages)
        await sim.event("listen", STOP_CHANNEL, "", OTHER, "stop")
        self.assertEqual(len(sim.messages), count)
        # malformed: a missing text, our own avatar
        await sim.event("http_request", OTHER, "POST", f"query|{sim.script.gToken}|pose|ask,{fifth},{OTHER},YQ==")
        self.assertEqual(sim.responses[-1][1], 400)
        await sim.ask(fifth, target=OWNER)
        self.assertEqual(sim.responses[-1][1], 400)

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
