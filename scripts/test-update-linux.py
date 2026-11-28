"""Linux twin of test-update.ps1: two throwaway packages, the next release
served on loopback, and an isolated installation launched on the first.

No tag nor release is published; the viewer refuses a normal launch from
this folder and keeps settings, cache and logs inside it. The feed stays up
while the viewer it launched, or the one the update restarted, runs, so a
check made after the restart reaches it like on a real installation.
"""

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
import uuid
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

FIRST = "0.2.100"
EXE = "aurora-viewer"  # the package's executable (aurora-updater's install::EXE)
QUIET = 3.0  # seconds without a process of this test before the feed stops


def run(*command, env=None, cwd=None):
    print("+", " ".join(str(part) for part in command), flush=True)
    subprocess.run(command, check=True, env=env, cwd=cwd)


def build(version):
    """The viewer at `version`, as a disposable fixture build."""
    env = os.environ | {"AURORA_UPDATE_TEST_VERSION": f"v{version}"}
    run("cargo", "build", "--locked", "-p", "aurora-viewer", env=env)


def package(repo, exe, out, version):
    """What package.ps1 writes, for the Linux package: the viewer, the
    licences of what is inside it, the desktop entry and icon, and the
    manifest the updater checks."""
    (out / "assets/emoji").mkdir(parents=True, exist_ok=True)
    (out / "assets/dictionaries").mkdir(parents=True, exist_ok=True)
    shutil.copy2(exe, out / EXE)
    shutil.copy2(repo / "assets/emoji/LICENSE-OFL.txt", out / "assets/emoji")
    dictionaries = repo / "crates/aurora-viewer/assets/dictionaries"
    shutil.copy2(dictionaries / "README.md", out / "assets/dictionaries")
    for folder in dictionaries.iterdir():
        licence = folder / "LICENSE.txt"
        if licence.is_file():
            shutil.copy2(licence, out / "assets/dictionaries" / f"{folder.name}-LICENSE.txt")
    for name in ("LICENSE", "NOTICE.md", "README.md"):
        shutil.copy2(repo / name, out)
    shutil.copy2(repo / "packaging/linux/aurora-viewer.desktop", out)
    shutil.copy2(repo / "assets/branding/aurora-logo.svg", out / "aurora-viewer.svg")
    (out / "aurora-install.json").write_text(
        json.dumps({"schema": 1, "version": version}), encoding="utf-8"
    )


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class Quiet(SimpleHTTPRequestHandler):
    """The feed's files under the test's root, without a line per request,
    except the release check and the archive: whether the viewer checked,
    and whether it downloaded."""

    def log_message(self, *args):
        pass

    def do_GET(self):
        if self.path.endswith((".tar.gz", "releases.json")):
            print(f"feed: GET {self.path}", flush=True)
        super().do_GET()


def serve(feed_dir):
    """The local feed, on loopback only, until this script exits."""
    handler = lambda *args, **kwargs: Quiet(  # noqa: E731
        *args, directory=feed_dir, **kwargs
    )
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def running(root):
    """The processes running from this test's root, by PID: the viewer, the
    one the update restarts, and the helper that swaps them — a copy of the
    viewer, started before the old one exits, so a swap never looks like an
    exit."""
    prefix = f"{root}{os.sep}"
    found = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            exe = os.readlink(entry / "exe")
        except OSError:
            continue  # exited meanwhile, or another account's process
        if exe.startswith(prefix):
            found[int(entry.name)] = exe
    return found


def follow(root, install, first):
    """The first viewer's status, then its restarted self until none has run
    for QUIET. A check made after the restart reaches the feed, as on a real
    installation; closing the viewer stops the script."""
    status = first.wait()
    restarted = False
    quiet = None
    while True:
        alive = running(root)
        if alive:
            quiet = None
            if not restarted and str(install / EXE) in alive.values():
                restarted = True
                print(
                    "The update restarted the viewer; the feed stays up until it is closed.",
                    flush=True,
                )
        elif quiet is None:
            quiet = time.monotonic()
        elif time.monotonic() - quiet >= QUIET:
            # Closed, or an update that never restarted the viewer (refused,
            # failed, cancelled): its own status stands.
            return 0 if restarted else status
        time.sleep(0.25)


def sign(key, path):
    """Signs `path` as scripts/sign-release.ps1 does: `<path>.sig`, the 64
    raw bytes of Ed25519 over the whole file."""
    run("openssl", "pkeyutl", "-sign", "-rawin", "-inkey", key, "-in", path, "-out", f"{path}.sig")


def new_key(path):
    """A key made for this test; returns its public half in hex (the last 32
    bytes of its DER form), which the isolated viewer trusts."""
    run("openssl", "genpkey", "-algorithm", "ed25519", "-out", path)
    public = subprocess.run(
        ["openssl", "pkey", "-in", path, "-pubout", "-outform", "DER"],
        check=True,
        capture_output=True,
    ).stdout
    return public[-32:].hex()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--title", default="Test viewer-update")
    parser.add_argument("--language", default="fr")
    parser.add_argument("--fps-limit", type=int)
    parser.add_argument(
        "--beta", action="store_true", help="serve the next version as a pre-release beta"
    )
    parser.add_argument("--clear-cache", action="store_true", help="the release invalidates the cache")
    parser.add_argument(
        "--reset-settings", action="store_true", help="the release resets the settings"
    )
    parser.add_argument("--required", action="store_true", help="the release is a required update")
    parser.add_argument(
        "--unsigned",
        action="store_true",
        help="the release is made by hand on GitHub, without signatures: never offered",
    )
    parser.add_argument(
        "--automatic", action="store_true", help="the Automatic update mode (downloads without asking)"
    )
    parser.add_argument(
        "--protected",
        action="store_true",
        help="the folder above the installation is read-only, as a system package's",
    )
    parser.add_argument(
        "--packaged",
        action="store_true",
        help="AURORA_PACKAGED=1, as a distribution package's launcher sets it",
    )
    options = parser.parse_args()

    repo = Path(
        subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    )
    os.chdir(repo)
    next_version = "0.2.101-beta.20261025.1" if options.beta else "0.2.101"
    root = repo / ".scratch" / f"update-test-{uuid.uuid4().hex}"
    feed = root / "feed"
    feed.mkdir(parents=True)
    install = root / "install"
    folder = f"Aurora-Viewer-v{next_version}-linux-x64"

    previous = os.environ.pop("AURORA_UPDATE_TEST_VERSION", None)
    try:
        build(FIRST)
        package(repo, repo / "target/debug/aurora-viewer", install, FIRST)
        build(next_version)
        package(repo, repo / "target/debug/aurora-viewer", feed / folder, next_version)
    finally:
        if previous is not None:
            os.environ["AURORA_UPDATE_TEST_VERSION"] = previous
    # the ordinary build back at its Git version
    run("cargo", "build", "--locked", "-p", "aurora-viewer")

    name = f"{folder}.tar.gz"
    run("tar", "-czf", name, folder, cwd=feed)
    archive = feed / name
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (feed / f"{name}.sha256").write_text(f"{digest}  {name}\n", encoding="utf-8")

    policy = {
        "schema": 1,
        "version": next_version,
        "minimum_supported_version": next_version if options.required else None,
        "cache_invalidation_before": next_version if options.clear_cache else None,
        "settings_reset_before": next_version if options.reset_settings else None,
    }
    (feed / "aurora-update-policy.json").write_text(json.dumps(policy), encoding="utf-8")
    # signed as the Release workflow signs them, with a key made for this
    # test: the isolated viewer trusts it from the loopback feed only
    key = root / "test-signing-key.pem"
    public_key = new_key(key)
    sign(key, feed / f"{name}.sha256")
    sign(key, feed / "aurora-update-policy.json")

    server = serve(str(feed))
    base = f"http://127.0.0.1:{server.server_port}"
    (root / "aurora-update-test.json").write_text(
        json.dumps({"schema": 1, "feed": f"{base}/releases.json", "public_key": public_key}),
        encoding="utf-8",
    )
    (root / "profile").mkdir()
    (root / "cache").mkdir()
    (root / "profile/settings.json").write_text(
        json.dumps(
            {
                "settings_version": 9,
                "language": options.language,
                "draw_distance": 300,
                "update_channel": "Beta" if options.beta else "Stable",
                "update_mode": "Automatic" if options.automatic else "Ask",
            }
        ),
        encoding="utf-8",
    )
    (root / "cache/update-marker.txt").write_text(
        "Isolated cache preservation proof\n", encoding="utf-8"
    )
    (root / "profile/update-marker.txt").write_text(
        "Isolated profile preservation proof\n", encoding="utf-8"
    )

    def asset(url_name, path):
        return {
            "name": url_name,
            "size": path.stat().st_size,
            "browser_download_url": f"{base}/{url_name}",
        }

    releases = [
        {
            "tag_name": f"v{next_version}",
            "draft": False,
            "prerelease": options.beta,
            "html_url": f"{base}/release.html",
            "body": "Trial of the real installation in an isolated folder.\n"
            "Maintenance policy announced before the restart.",
            "assets": [
                asset(name, archive),
                asset(f"{name}.sha256", feed / f"{name}.sha256"),
                asset("aurora-update-policy.json", feed / "aurora-update-policy.json"),
            ]
            + (
                []
                if options.unsigned
                else [
                    asset(f"{name}.sha256.sig", feed / f"{name}.sha256.sig"),
                    asset("aurora-update-policy.json.sig", feed / "aurora-update-policy.json.sig"),
                ]
            ),
        }
    ]
    (feed / "releases.json").write_text(json.dumps(releases), encoding="utf-8")
    (feed / "release.html").write_text(
        "<!doctype html><meta charset=\"utf-8\"><title>Test release</title>"
        f"<p>Local test release of Aurora {next_version}.</p>",
        encoding="utf-8",
    )

    environment = os.environ | {"AURORA_UPDATE_TEST_ROOT": str(root)}
    environment.pop("AURORA_DEMO", None)
    environment.pop("AURORA_PACKAGED", None)
    if options.packaged:
        environment["AURORA_PACKAGED"] = "1"
    if options.protected:
        # what the viewer writes under the test's root exists beforehand;
        # nothing new can be made beside the installation
        for name in ("logs", "media-plugins"):
            (root / name).mkdir(exist_ok=True)
    arguments = [str(install / EXE), "--title", options.title, "--language", options.language]
    if options.fps_limit is not None:
        arguments += ["--fps-limit", str(options.fps_limit)]
    viewer = None
    try:
        if options.protected:
            root.chmod(0o555)
        viewer = subprocess.Popen(arguments, cwd=install, env=environment)
        print(f"Test installation: {root}", flush=True)
        print(f"Viewer: {viewer.pid}; local feed: {base}", flush=True)
        return follow(root, install, viewer)
    except KeyboardInterrupt:
        if viewer is None:
            return 130
        viewer.terminate()
        return viewer.wait()
    finally:
        if options.protected:
            root.chmod(0o755)  # or the test's folder cannot be deleted


if __name__ == "__main__":
    sys.exit(main())
