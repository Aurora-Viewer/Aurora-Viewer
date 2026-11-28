"""Build the script editor's LSL definitions table.

Input: lsl_definitions.yaml of secondlife/lsl-definitions (MIT, Copyright (c)
2025 Linden Research, Inc.), at the release named below. Two things are read
from it, and only those:
  - the `deprecated:` blocks: the name each deprecated function, constant,
    event or control has to use instead;
  - the `arguments:` of every function and event: the name and the meaning of
    each parameter, in the order the signature writes them.

Output: crates/aurora-viewer/assets/lsl/definitions.json, read by
aurora-viewer/src/lsl_keywords.rs (keep the two in step): the deprecated
names, and the arguments the help and the inlay hints describe. Built into the
executable.

    python scripts/generate-lsl-definitions.py
    python scripts/generate-lsl-definitions.py --yaml path/to/lsl_definitions.yaml

Without --yaml the file is downloaded from the pinned commit. The table states
the commit, the file and its SHA-256, so a refresh is one line and a diff.
The definitions are not the compile contract: the region's syntax stays what
the editor highlights, completes and lints with; the table only decorates the
names a catalog already has.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import urllib.request
from pathlib import Path

REPO = "https://github.com/secondlife/lsl-definitions"
TAG = "v0.7.2.1-die-no-rezzer"
COMMIT = "a4be83c390aa60cef5f7066cd2fcb7ee9823eaf6"
FILE = "lsl_definitions.yaml"
OUT = Path(__file__).resolve().parent.parent / "crates/aurora-viewer/assets/lsl/definitions.json"

# The names the table must hold: a silent change of the file's shape would
# otherwise make the help say nothing where it has something to say.
KNOWN_DEPRECATED = {
    "llSetPrimitiveParams": "ll.SetLinkPrimitiveParamsFast",
    "llSound": "ll.PlaySound",
    "llCloud": None,
    "ATTACH_LPEC": "ATTACH_RIGHT_PEC",
}
KNOWN_ARGUMENTS = {
    "llSay": ["channel", "msg"],
    "llListen": ["channel", "name", "id", "msg"],
    "llSetText": ["text", "color", "alpha"],
}

BLOCK = ("|", "|-", "|+", ">", ">-", ">+")


def read_arguments(lines: list[str], at: int) -> tuple[list[list[str]], int]:
    """The arguments of a function or event, from the line after `arguments:`.

    Each is `    - name:` then fields at eight spaces; a plain scalar folds its
    continuation lines (ten spaces) into one text, a block scalar keeps its
    lines. Anything else in the block (types, semantics) is left out.
    """
    arguments: list[list[str]] = []
    while at < len(lines):
        item = re.match(r"^    - ([\w.]+):\s*$", lines[at])
        if not item:
            break
        name = item.group(1)
        at += 1
        field: dict[str, str] = {}
        while at < len(lines) and lines[at].startswith("        "):
            head = re.match(r"^        ([\w-]+):\s*(.*)$", lines[at])
            if not head:
                at += 1
                continue
            key, value = head.group(1), head.group(2).strip()
            at += 1
            if value in BLOCK:
                body = []
                while at < len(lines) and (lines[at].startswith("          ") or not lines[at].strip()):
                    body.append(lines[at][10:] if lines[at].startswith("          ") else "")
                    at += 1
                value = ("\n" if value.startswith("|") else " ").join(body).strip()
            else:
                continuation = []
                while at < len(lines) and re.match(r"^          \S", lines[at]):
                    continuation.append(lines[at].strip())
                    at += 1
                if continuation:
                    value = " ".join([value, *continuation]).strip()
                if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
                    value = value[1:-1]
            field[key] = value
        tooltip = field.get("tooltip", "").strip()
        if tooltip in ("''", '""'):
            tooltip = ""
        arguments.append([name, tooltip])
    return arguments, at


def read(yaml: str) -> tuple[dict[str, str | None], dict[str, list[list[str]]]]:
    """The deprecated names, and the arguments of every function and event."""
    lines = yaml.splitlines()
    deprecated: dict[str, str | None] = {}
    arguments: dict[str, list[list[str]]] = {}
    section = None
    entry = None
    at = 0
    while at < len(lines):
        line = lines[at]
        top = re.match(r"^([a-zA-Z][\w-]*):", line)
        if top:
            section = top.group(1)
            entry = None
            at += 1
            continue
        named = re.match(r"^  ([\w.]+):\s*$", line)
        if named:
            entry = named.group(1)
            at += 1
            continue
        if entry and re.match(r"^    arguments:", line):
            if section in ("functions", "events"):
                found, at = read_arguments(lines, at + 1)
                if found:
                    arguments[entry] = found
            else:
                at += 1
            continue
        if entry and re.match(r"^    deprecated:", line):
            use = None
            at += 1
            while at < len(lines) and lines[at].startswith("      "):
                replacement = re.match(r"^      use:\s*(\S+)", lines[at])
                if replacement:
                    use = replacement.group(1)
                at += 1
            if entry in deprecated:
                raise SystemExit(f"{entry} is deprecated twice")
            deprecated[entry] = use
            continue
        at += 1
    return deprecated, arguments


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--yaml", type=Path, help="a local lsl_definitions.yaml instead of the pinned download")
    parser.add_argument("--out", type=Path, default=OUT, help=f"where the table goes (default: {OUT})")
    args = parser.parse_args()

    if args.yaml:
        data = args.yaml.read_bytes()
    else:
        url = f"https://raw.githubusercontent.com/secondlife/lsl-definitions/{COMMIT}/{FILE}"
        with urllib.request.urlopen(url, timeout=120) as reply:
            data = reply.read()

    deprecated, arguments = read(data.decode("utf-8"))
    if len(deprecated) < 50:
        raise SystemExit(f"only {len(deprecated)} deprecated names: the file's layout has changed")
    for name, want in KNOWN_DEPRECATED.items():
        if name not in deprecated:
            raise SystemExit(f"{name} is missing from the deprecated names")
        if deprecated[name] != want:
            raise SystemExit(f"{name} now uses {deprecated[name]!r}, not {want!r}")
    count = sum(len(found) for found in arguments.values())
    if count < 850:
        raise SystemExit(f"only {count} documented arguments: the file's layout has changed")
    for name, want in KNOWN_ARGUMENTS.items():
        got = [arg[0] for arg in arguments.get(name, [])]
        if got != want:
            raise SystemExit(f"{name} now takes {got}, not {want}")
    for name, found in arguments.items():
        if any(not arg[0] for arg in found):
            raise SystemExit(f"{name} has an argument without a name")

    document = {
        "source": {
            "repo": REPO,
            "tag": TAG,
            "commit": COMMIT,
            "file": FILE,
            "sha256": hashlib.sha256(data).hexdigest(),
            "license": "MIT",
            "copyright": "Copyright (c) 2025 Linden Research, Inc.",
        },
        "deprecated": dict(sorted(deprecated.items())),
        "arguments": dict(sorted(arguments.items())),
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")
    replaced = sum(1 for use in deprecated.values() if use)
    print(f"{args.out}: {len(deprecated)} deprecated names ({replaced} with a replacement), {count} arguments in {len(arguments)} calls")


if __name__ == "__main__":
    main()
