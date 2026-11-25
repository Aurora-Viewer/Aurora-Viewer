"""A real input method against the viewer, on Linux, without touching the
desktop: a headless sway, fcitx5 with Mozc on a private D-Bus, and the demo
viewer in it, its keys typed by wtype.

Each scenario starts its own compositor, input method and viewer, types
Japanese as a user does ("mama", F7 for katakana, Enter to commit) and reads
the result from the viewer's log:

- inventory (AURORA_DEMO_INVENTORY=ime, the Legacy tree) and
  aurora-inventory (=aurora-ime, the explorer): the list holds the keyboard
  after a click on Animations. "z" with the input method off selects
  "Zipper bag"; while composing, the selection and the list's keyboard stay
  put; the committed "ママ" selects "ママ服", not "マスク".
- chat (AURORA_DEMO_CHAT_INPUT=os-ime): the empty Conversations field holds
  the keyboard; "a" then BackSpace with the input method off, then the
  commit "ママ" lands in the field.

In every scenario the viewer must never ask the input method to drop the
composition (`ime=interrupt` in its log) once the target holds the keyboard.

Needs: sway, fcitx5 with fcitx5-mozc, wtype, dbus-daemon, setpriv
(util-linux), and grim for the captures (optional). The viewer's settings,
the input method's profile and Mozc's dictionary stay in the output folder;
nothing of the user's session is read or written. Exit code 0 when every
check passed, 1 when one failed, 2 when the test could not run.
"""

import argparse
import datetime
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TITLE = "IME test [AUTOMATION - DO NOT TOUCH]"
SCENARIOS = ("inventory", "aurora-inventory", "chat")

# The demo rows (crates/aurora-viewer/src/world/inventory/demo.rs).
MASK = 9701  # マスク
MAMA = 9702  # ママ服
ZIPPER = 9703  # Zipper bag

SWAY_CONFIG = "output HEADLESS-1 resolution 1600x1000\n"

# A session bus of our own with no service folder, so that nothing on it is
# activated: a desktop portal started there would attach to the user's
# display.
BUS_CONFIG = """<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:dir={dir}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"""

FCITX_PROFILE = """[Groups/0]
Name=Default
Default Layout=us
DefaultIM=mozc

[Groups/0/Items/0]
Name=keyboard-us
Layout=

[Groups/0/Items/1]
Name=mozc
Layout=

[GroupOrder]
0=Default
"""

INVENTORY_LINE = re.compile(
    r"demo inventory ime: frame=(\d+), selected=(?:Some\((\d+)\)|None), "
    r'prefix="(.*)", list_focus=(true|false)'
)
CHAT_LINE = re.compile(r'demo chat os-ime: frame=(\d+), text="(.*)", field_focus=(true|false)')
IME_LINE = re.compile(r"demo ime output: frame=(\d+), ime=(\w+)")


class CannotRun(Exception):
    """The test could not run: a missing tool, or a process that did not start."""


class Failed(Exception):
    """A check failed."""


def missing_tools(captures):
    tools = ["sway", "fcitx5", "fcitx5-remote", "wtype", "dbus-daemon", "setpriv"]
    if captures:
        tools.append("grim")
    missing = [tool for tool in tools if shutil.which(tool) is None]
    data_dirs = os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
    if not any(Path(base, "fcitx5/inputmethod/mozc.conf").is_file() for base in data_dirs.split(":")):
        missing.append("fcitx5-mozc")
    return missing


def build():
    """The debug viewer, built from this checkout."""
    print("+ cargo build --locked -p aurora-viewer", flush=True)
    subprocess.run(["cargo", "build", "--locked", "-p", "aurora-viewer"], check=True, cwd=REPO)
    meta = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps"],
        check=True,
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    return Path(json.loads(meta.stdout)["target_directory"]) / "debug" / "aurora-viewer"


def wait(what, check, timeout):
    """Poll `check` until it returns something true and return that."""
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = check()
        if value:
            return value
        time.sleep(0.2)
    raise Failed(f"timed out after {timeout:.0f} s waiting for {what}")


class Session:
    """sway, a private bus, fcitx5 and one viewer, their files in `out`."""

    def __init__(self, out, viewer, demo_env, captures):
        self.out = out
        self.viewer = viewer
        self.demo_env = demo_env
        self.captures = captures
        self.procs = []
        self.log = out / "viewer.log"
        self.viewer_proc = None
        self.shot = 0
        self.runtime = None
        self.fcitx_home = out / "fcitx"

    def __enter__(self):
        try:
            self.start()
        except BaseException:
            self.__exit__(None, None, None)
            raise
        return self

    def start(self):
        # A Unix socket's path holds 108 bytes at most: the runtime folder
        # gets a short name under the user's own.
        base = os.environ.get("XDG_RUNTIME_DIR") or tempfile.gettempdir()
        self.runtime = Path(tempfile.mkdtemp(prefix="aime", dir=base))
        (self.fcitx_home / "fcitx5").mkdir(parents=True)
        (self.fcitx_home / "fcitx5/profile").write_text(FCITX_PROFILE, encoding="utf-8")
        (self.out / "sway.conf").write_text(SWAY_CONFIG, encoding="utf-8")
        (self.out / "bus.conf").write_text(BUS_CONFIG.format(dir=self.runtime), encoding="utf-8")

        outside = {
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "WAYLAND_SOCKET",
            "SWAYSOCK",
            "HYPRLAND_INSTANCE_SIGNATURE",
            "DBUS_SESSION_BUS_ADDRESS",
            "GTK_IM_MODULE",
            "QT_IM_MODULE",
            "XMODIFIERS",
        }
        env = {key: value for key, value in os.environ.items() if key not in outside}
        env |= {
            "XDG_RUNTIME_DIR": str(self.runtime),
            "XDG_CURRENT_DESKTOP": "sway",
            "WLR_BACKENDS": "headless",
            "WLR_LIBINPUT_NO_DEVICES": "1",
        }
        bus = self.spawn(
            "dbus",
            ["dbus-daemon", "--nofork", "--print-address=1", f"--config-file={self.out / 'bus.conf'}"],
            env,
            stdout=subprocess.PIPE,
        )
        address = bus.stdout.readline().decode().strip()
        if not address:
            raise CannotRun(f"the private D-Bus did not start (see {self.out / 'dbus.log'})")
        env["DBUS_SESSION_BUS_ADDRESS"] = address

        # sway's file capability (cap_sys_nice) has it ask for realtime
        # scheduling, which a shell whose RLIMIT_RTTIME is 0 answers with
        # SIGKILL; with no new privileges the capability is not applied.
        self.spawn("sway", ["setpriv", "--no-new-privs", "sway", "-c", str(self.out / "sway.conf")], env)

        def socket():
            return next((p.name for p in self.runtime.glob("wayland-*") if not p.name.endswith(".lock")), None)

        try:
            env["WAYLAND_DISPLAY"] = wait("sway's socket", socket, 15)
        except Failed as error:
            raise CannotRun(f"sway did not start (see {self.out / 'sway.log'})") from error
        self.env = env

        # A headless seat has a keyboard only while a virtual keyboard
        # exists: each wtype call would add and remove one, and the viewer
        # would lose the keyboard's focus between two keys. One wtype that
        # waits holds the keyboard for the whole session.
        self.spawn("wtype-hold", ["wtype", "-s", "3600000", "-m", "shift"], env)
        time.sleep(0.3)
        self.spawn("fcitx5", ["fcitx5", "--verbose=default=3"], env | {"XDG_CONFIG_HOME": str(self.fcitx_home)})
        try:
            wait("fcitx5 on the private bus", lambda: self.remote() is not None, 15)
        except Failed as error:
            raise CannotRun(f"fcitx5 did not start (see {self.out / 'fcitx5.log'})") from error

        viewer_env = env | self.demo_env
        viewer_env |= {
            "AURORA_DEMO": "1",
            "AURORA_SPLASH": "0",
            "AURORA_DEMO_MEDIA_PACK": "declined",
            "XDG_CONFIG_HOME": str(self.out / "config"),
            "XDG_DATA_HOME": str(self.out / "data"),
            "XDG_CACHE_HOME": str(self.out / "cache"),
        }
        self.viewer_proc = self.spawn("viewer", [str(self.viewer), "--title", TITLE], viewer_env)

    def spawn(self, name, command, env, stdout=None):
        handle = open(self.out / f"{name}.log", "wb")
        proc = subprocess.Popen(
            command,
            env=env,
            cwd=REPO,
            stdin=subprocess.DEVNULL,
            stdout=stdout or handle,
            stderr=handle,
            start_new_session=True,
        )
        self.procs.append((proc, handle))
        return proc

    def remote(self, *args):
        """fcitx5-remote's answer, or None while fcitx5 does not answer."""
        result = subprocess.run(["fcitx5-remote", *args], env=self.env, capture_output=True, text=True)
        return result.stdout.strip() if result.returncode == 0 else None

    def keys(self, *args):
        subprocess.run(["wtype", *args], env=self.env, check=True)

    def capture(self, name):
        if self.captures:
            self.shot += 1
            subprocess.run(["grim", str(self.out / f"{self.shot:02d}-{name}.png")], env=self.env, check=False)

    def lines(self, pattern):
        if self.viewer_proc.poll() is not None:
            raise Failed(f"the viewer exited with code {self.viewer_proc.returncode} (see {self.log})")
        text = self.log.read_text(encoding="utf-8", errors="replace") if self.log.exists() else ""
        return [m.groups() for m in pattern.finditer(text)]

    def __exit__(self, *exc):
        for proc, _ in reversed(self.procs):
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGTERM)
        for proc, handle in self.procs:
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
            handle.close()
        # fcitx5-mozc starts mozc_server, which outlives fcitx5: ours is the
        # one whose configuration folder is this session's.
        mark = f"XDG_CONFIG_HOME={self.fcitx_home}".encode()
        for proc_dir in Path("/proc").iterdir():
            try:
                if (proc_dir / "comm").read_text().strip() == "mozc_server" and mark in (
                    proc_dir / "environ"
                ).read_bytes().split(b"\0"):
                    os.kill(int(proc_dir.name), signal.SIGTERM)
            except (OSError, ValueError):
                pass
        if self.runtime:
            shutil.rmtree(self.runtime, ignore_errors=True)


def mozc_on(s):
    s.remote("-o")
    wait("Mozc to be the active input method", lambda: s.remote("-n") == "mozc", 10)


def mozc_off(s):
    s.remote("-c")
    wait("the input method to be off", lambda: s.remote("-n") not in (None, "mozc"), 10)


def interrupts(s, since):
    """A note for a failure message when the viewer dropped compositions."""
    count = sum(1 for frame, ime in s.lines(IME_LINE) if ime == "interrupt" and int(frame) >= since)
    return f" (the viewer asked the input method to drop the composition {count} times)" if count else ""


def no_interrupt(s, since):
    frames = [int(frame) for frame, ime in s.lines(IME_LINE) if ime == "interrupt" and int(frame) >= since]
    if frames:
        raise Failed(f"the viewer asked the input method to drop the composition at frames {frames}")
    if not any(ime == "on" for frame, ime in s.lines(IME_LINE) if int(frame) >= since):
        raise Failed("the viewer never enabled the input method")


def inventory(s):
    def selected(row=None):
        """The last log line once `row` (any row when None) is selected
        and the list holds the keyboard."""

        def check():
            lines = s.lines(INVENTORY_LINE)
            last = lines[-1] if lines else None
            return last and last[1] and (row is None or last[1] == str(row)) and last[3] == "true" and last

        return check

    focus = wait("the click on Animations (the list holding the keyboard)", selected(), 180)
    since = int(focus[0])
    s.capture("start")

    mozc_off(s)
    s.keys("z")
    wait("'z' (input method off) to select Zipper bag", selected(ZIPPER), 10)
    print("  ok: with the input method off, 'z' selects Zipper bag")
    time.sleep(2)  # past the 1.5 s type-ahead timeout: "ママ" starts a new prefix

    mozc_on(s)
    s.keys("-d", "80", "mama")
    time.sleep(1.5)
    s.capture("composing")
    s.keys("-k", "F7")
    time.sleep(1.5)
    s.capture("katakana")
    last = s.lines(INVENTORY_LINE)[-1]
    if last[1] != str(ZIPPER) or last[3] != "true":
        raise Failed(f"the selection or the list's keyboard moved while composing: {last}")
    print("  ok: while composing, Zipper bag stays selected and the list keeps the keyboard")

    s.keys("-k", "Return")
    try:
        done = wait('the commit "ママ" to select ママ服', selected(MAMA), 10)
    except Failed:
        lines = s.lines(INVENTORY_LINE)
        last = lines[-1] if lines else None
        cut = " (マスク: the composition was cut short)" if last and last[1] == str(MASK) else ""
        raise Failed(f'the commit "ママ" did not select ママ服{cut}{interrupts(s, since)}; last log: {last}') from None
    s.capture("committed")
    if done[2] != "ママ":
        raise Failed(f'the prefix is {done[2]!r}, not "ママ"')
    print('  ok: the commit "ママ" selects ママ服')
    no_interrupt(s, since)
    print("  ok: the input method was on and never told to drop the composition")


def chat(s):
    def text(value):
        def check():
            lines = s.lines(CHAT_LINE)
            return lines and lines[-1][1] == value and lines[-1][2] == "true" and lines[-1]

        return check

    focus = wait("the Conversations field holding the keyboard", text(""), 180)
    since = int(focus[0])
    s.capture("start")

    mozc_off(s)
    s.keys("a")
    wait("'a' (input method off) in the field", text("a"), 10)
    s.keys("-k", "BackSpace")
    wait("BackSpace to empty the field", text(""), 10)
    print("  ok: with the input method off, keys reach the field")

    mozc_on(s)
    s.keys("-d", "80", "mama")
    time.sleep(1.5)
    s.capture("composing")
    s.keys("-k", "F7")
    time.sleep(1.5)
    s.capture("katakana")
    s.keys("-k", "Return")
    try:
        wait('the commit "ママ" in the field', text("ママ"), 10)
    except Failed as error:
        raise Failed(f"{error}{interrupts(s, since)}") from None
    s.capture("committed")
    print('  ok: the commit "ママ" lands in the field')
    no_interrupt(s, since)
    print("  ok: the input method was on and never told to drop the composition")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument(
        "--only", choices=SCENARIOS, action="append", help="run this scenario (repeatable); all three by default"
    )
    parser.add_argument("--viewer", type=Path, help="this viewer binary instead of a cargo build")
    parser.add_argument("--out", type=Path, help="the output folder (default .scratch/ime-test/<time>)")
    parser.add_argument("--no-captures", action="store_true", help="no grim screenshots")
    args = parser.parse_args()

    missing = missing_tools(not args.no_captures)
    if missing:
        print("missing: " + ", ".join(missing) + " (see docs/TESTING.md, AURORA_DEMO_INVENTORY=ime)", file=sys.stderr)
        return 2
    out = (args.out or REPO / ".scratch/ime-test" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S")).resolve()
    if out.exists() and any(out.iterdir()):
        # An earlier run's viewer.log would be read as this run's.
        print(f"{out} is not empty: give a new --out", file=sys.stderr)
        return 2
    try:
        viewer = (args.viewer or build()).resolve()
    except subprocess.CalledProcessError as error:
        print(f"the build failed: {error}", file=sys.stderr)
        return 2
    if not viewer.is_file():
        print(f"no viewer at {viewer}", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)

    plans = {
        "inventory": ({"AURORA_DEMO_INVENTORY": "ime"}, inventory),
        "aurora-inventory": ({"AURORA_DEMO_INVENTORY": "aurora-ime"}, inventory),
        "chat": ({"AURORA_DEMO_CHAT_INPUT": "os-ime"}, chat),
    }
    failures = 0
    for name in args.only or SCENARIOS:
        demo_env, scenario = plans[name]
        folder = out / name
        folder.mkdir()
        print(f"{name}:", flush=True)
        try:
            with Session(folder, viewer, demo_env, not args.no_captures) as s:
                scenario(s)
        except CannotRun as error:
            print(f"  cannot run: {error}", file=sys.stderr)
            return 2
        except Failed as error:
            failures += 1
            print(f"  FAILED: {error}", flush=True)
    print(f"{f'{failures} failed' if failures else 'all passed'}; logs and captures in {out}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
