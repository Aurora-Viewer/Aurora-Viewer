#!/usr/bin/env python3
"""Aurora Weather: the GPU cost of each weather feature, measured.

Runs the release viewer in demo mode once per scene (the same place, camera
and sun each time, the weather set by the AURORA_* variables of
docs/TESTING.md, "Weather"), with AURORA_PROFILE=1, for a few seconds each,
then reads each scene's `perf summary` lines and prints a Markdown table:
the frame, the GPU time, the weather's (`g_weather`) and its parts, against
the scene without weather.

Usage (from the repository root, after `cargo build --release -p aurora-viewer`):

    python3 scripts/weather-profile.py                  # every scene, 20 s each
    python3 scripts/weather-profile.py --seconds 30 --scenes base,rain-heavy
    python3 scripts/weather-profile.py --parse-only     # the last runs' logs again

Every run has the same window size and graphics preset (1920 x 1080 and
High by default: `--size`, `--preset`), whatever the screen and the demo's
saved settings: the weather's full-screen passes cost with the pixels. The
weather variables a scene leaves out are set to none, so that no weather
saved in the demo's settings is measured instead.

Measure on a quiet machine: nothing else drawing (no other viewer open),
the window in front. Numbers from a software renderer (lavapipe) say
nothing about a GPU.
"""

import json

import argparse
import os
import platform
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

NIGHT = {"AURORA_DEMO_NIGHT_SKY": "real:0,20"}

# name: (what it measures, variables)
SCENES = {
    "base": ("no weather", {}),
    "rain-light": ("light rain", {"AURORA_RAIN": "0.25"}),
    "rain-heavy": ("heavy rain", {"AURORA_RAIN": "1"}),
    "rain-heavy-low": ("heavy rain, Low", {"AURORA_RAIN": "1", "AURORA_WEATHER_QUALITY": "low"}),
    "rain-heavy-medium": ("heavy rain, Medium", {"AURORA_RAIN": "1", "AURORA_WEATHER_QUALITY": "medium"}),
    "thunder": ("heavy rain, a strike held", {"AURORA_RAIN": "1", "AURORA_THUNDER_AT": "3", "AURORA_THUNDER_HOLD": "1"}),
    "snow": ("snow falling", {"AURORA_SNOW": "0.7", "AURORA_SNOW_LYING": "0"}),
    "snow-lying": ("snow lying", {"AURORA_SNOW": "0.7", "AURORA_SNOW_LYING": "1"}),
    "snow-deep": ("deep snow", {"AURORA_SNOW": "0.7", "AURORA_SNOW_LYING": "1", "AURORA_SNOW_DEEP": "1"}),
    "snow-lying-low": ("snow lying, Low", {"AURORA_SNOW": "0.7", "AURORA_SNOW_LYING": "1", "AURORA_WEATHER_QUALITY": "low"}),
    "night": ("night, no aurora", dict(NIGHT)),
    "aurora": ("northern lights", {**NIGHT, "AURORA_NORTHERN_LIGHTS": "0.9,0,1"}),
}

# what a scene does not set: no weather (the demo's saved settings may hold some)
NEUTRAL = {"AURORA_RAIN": "0", "AURORA_SNOW_DEEP": "0", "AURORA_WEATHER_QUALITY": "high"}

# the columns, from each summary line
KEYS = ["fps", "frame", "gpu", "g_weather", "g_shelter", "g_weather_memory", "g_wet_ground", "g_drops", "g_terrain_sky"]
# the first summaries: loading, shaders, the snow memory filling
WARMUP = 6
NUMBER = re.compile(r"(\w+)=(-?\d+(?:\.\d+)?)")


def logs_dir() -> Path:
    """The viewer's logs directory (logging.rs `logs_dir`)."""
    home = Path.home()
    system = platform.system()
    if system == "Windows":
        return Path(os.environ.get("LOCALAPPDATA", home / "AppData/Local")) / "Aurora/AuroraViewer/data/logs"
    if system == "Darwin":
        return home / "Library/Application Support/org.Aurora.AuroraViewer/logs"
    return Path(os.environ.get("XDG_DATA_HOME", home / ".local/share")) / "auroraviewer/logs"


def default_exe() -> Path:
    name = "aurora-viewer.exe" if platform.system() == "Windows" else "aurora-viewer"
    return Path("target/release") / name


def log_name(scene: str) -> str:
    return f"weather-profile-{scene}"


def demo_settings_warning() -> str | None:
    """The demo's settings letting a region set the weather (it would win
    over the scene's variables)."""
    system = platform.system()
    home = Path.home()
    if system == "Windows":
        base = Path(os.environ.get("APPDATA", home / "AppData/Roaming")) / "Aurora/AuroraViewer/config"
    elif system == "Darwin":
        base = home / "Library/Application Support/org.Aurora.AuroraViewer"
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME", home / ".config")) / "auroraviewer"
    path = base / "demo" / "settings.json"
    try:
        weather = json.loads(path.read_text(encoding="utf-8")).get("weather", {})
    except (OSError, ValueError):
        return None
    if weather.get("region_weather"):
        return f"{path}: \"Let regions set the weather\" is on; turn it off in the Aurora Weather window first"
    return None


def run(exe: Path, scene: str, seconds: float, logs: Path, size: str, preset: str) -> None:
    _, variables = SCENES[scene]
    env = dict(os.environ)
    # nothing left over from the shell's own tests
    for k in list(env):
        if k.startswith(("AURORA_RAIN", "AURORA_SNOW", "AURORA_THUNDER", "AURORA_NORTHERN", "AURORA_WEATHER", "AURORA_DEMO_NIGHT")):
            del env[k]
    env.update({"AURORA_DEMO": "1", "AURORA_PROFILE": "1", "AURORA_LOG_NAME": log_name(scene)})
    env.update({"AURORA_WINDOW_SIZE": size, "AURORA_GRAPHICS_PRESET": preset})
    env.update(NEUTRAL)
    env.update(variables)
    # this run's numbers only
    (logs / f"{log_name(scene)}.log").unlink(missing_ok=True)
    print(f"  {scene}: {seconds:.0f} s", flush=True)
    proc = subprocess.Popen([str(exe), "--title", f"Profile {scene}"], env=env)
    try:
        time.sleep(seconds)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
    # the log is flushed once a second
    time.sleep(1.5)


def read(path: Path) -> dict:
    """The median of each key over the scene's summaries, warm-up left out."""
    values = {k: [] for k in KEYS}
    lines = [l for l in path.read_text(encoding="utf-8", errors="replace").splitlines() if "perf summary:" in l]
    for line in lines[WARMUP:]:
        found = dict(NUMBER.findall(line.split("perf summary:", 1)[1]))
        for k in KEYS:
            if k in found:
                values[k].append(float(found[k]))
    return {k: statistics.median(v) if v else None for k, v in values.items()} | {"samples": max(0, len(lines) - WARMUP)}


def table(results: dict) -> str:
    def cell(v, digits=2):
        return "–" if v is None else f"{v:.{digits}f}"

    base = results.get("base", {}).get("gpu")
    night = results.get("night", {}).get("gpu")
    head = "| Scene | fps | frame ms | GPU ms | vs base | weather | shelter | memory | wet ground | drops | samples |"
    out = [head, "|---" * 11 + "|"]
    for scene, r in results.items():
        ref = night if scene == "aurora" else base
        delta = None if r.get("gpu") is None or ref is None else r["gpu"] - ref
        what = SCENES[scene][0]
        out.append(
            f"| {what} | {cell(r['fps'], 0)} | {cell(r['frame'])} | {cell(r['gpu'])} | "
            f"{'–' if delta is None else f'{delta:+.2f}'} | {cell(r['g_weather'], 3)} | {cell(r['g_shelter'], 3)} | "
            f"{cell(r['g_weather_memory'], 3)} | {cell(r['g_wet_ground'], 3)} | {cell(r['g_drops'], 3)} | {r['samples']} |"
        )
    out.append("")
    out.append("Medians over each scene's seconds (the first ones left out). \"vs base\": GPU time against the scene")
    out.append("without weather (the northern lights: against the same night without them; they are in `g_terrain_sky`).")
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--exe", type=Path, default=default_exe())
    ap.add_argument("--seconds", type=float, default=20.0)
    ap.add_argument("--scenes", default=",".join(SCENES), help="comma-separated, among: " + ", ".join(SCENES))
    ap.add_argument("--log-dir", type=Path, default=None)
    ap.add_argument("--size", default="1920,1080", help="window size, w,h (default 1920,1080)")
    ap.add_argument("--preset", default="high", help="graphics preset: potato, low, medium, high, deluxe (default high)")
    ap.add_argument("--parse-only", action="store_true", help="read the logs of earlier runs, run nothing")
    args = ap.parse_args()

    scenes = [s.strip() for s in args.scenes.split(",") if s.strip()]
    unknown = [s for s in scenes if s not in SCENES]
    if unknown:
        print(f"unknown scenes: {', '.join(unknown)}", file=sys.stderr)
        return 2
    logs = args.log_dir or logs_dir()
    if not args.parse_only:
        if not args.exe.exists():
            print(f"{args.exe} not found: cargo build --release -p aurora-viewer", file=sys.stderr)
            return 2
        warning = demo_settings_warning()
        if warning:
            print(warning, file=sys.stderr)
            return 2
        print(f"Running {len(scenes)} scenes at {args.size}, preset {args.preset}; leave the windows alone.")
        for s in scenes:
            run(args.exe, s, args.seconds, logs, args.size, args.preset)
    results = {}
    for s in scenes:
        path = logs / f"{log_name(s)}.log"
        if not path.exists():
            print(f"no log for {s}: {path}", file=sys.stderr)
            continue
        results[s] = read(path)
    if not results:
        return 1
    print()
    print(f"Window {args.size}, preset {args.preset}.")
    print()
    print(table(results))
    return 0


if __name__ == "__main__":
    sys.exit(main())
