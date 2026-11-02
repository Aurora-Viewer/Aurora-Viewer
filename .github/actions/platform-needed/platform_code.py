"""Does a change touch Rust code of a platform's own?

Usage: platform_code.py <macos|linux> <base> <head>

Prints the first file whose change is platform code and exits 0, or exits 1
when none is. A changed line counts when it, a line near it, or one of the
items around it (function, impl, mod, found by rustfmt's indentation) carries
a cfg naming the platform (or excluding another system, so that the
platform builds it), or when the file itself is a module declared under such
a cfg. Lines are read in the new version and removed lines in the old one.
A change elsewhere that a cfg'd caller depends on is what the run on main
catches.
"""

import os
import re
import subprocess
import sys

PATTERNS = {
    "macos": r"cfg!?\([^)]*(macos|apple|unix|not\((windows|any|target_os))",
    "linux": r"cfg!?\([^)]*(linux|unix|freebsd|not\((windows|any|target_os))",
}
NEAR = 5


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace").stdout


def lines_of(rev, path):
    out = subprocess.run(["git", "show", f"{rev}:{path}"], capture_output=True, text=True, encoding="utf-8", errors="replace")
    return out.stdout.split("\n") if out.returncode == 0 else []


def indent(line):
    return len(line) - len(line.lstrip(" "))


def gated(lines, i, cfg):
    """Line i (0-based) is platform code: near a cfg, or inside an item
    whose attributes hold one."""
    lo, hi = max(0, i - NEAR), min(len(lines), i + NEAR + 1)
    if any(cfg.search(l) for l in lines[lo:hi]):
        return True
    level = indent(lines[i]) if lines[i].strip() else 10**6
    j = i - 1
    while j >= 0 and level > 0:
        line = lines[j]
        if line.strip() and indent(line) < level and not line.lstrip().startswith(("//", "#[", "}", ")", "]")):
            level = indent(line)
            # the attributes and doc comments right above this header
            k = j - 1
            while k >= 0 and lines[k].lstrip().startswith(("#[", "#![", "///", "//")):
                if cfg.search(lines[k]):
                    return True
                k -= 1
        elif line.lstrip().startswith("#![") and cfg.search(line):
            return True
        j -= 1
    return False


def module_gated(rev, path, cfg, seen=0):
    """The file is a module declared under a cfg in its parent (or a parent
    of it is)."""
    if seen > 6:
        return False
    head, name = os.path.split(path)
    stem = name[:-3]
    if stem in ("lib", "main"):
        return False
    if stem == "mod":
        head, stem = os.path.split(head)
    parents = [os.path.join(head, "mod.rs"), os.path.join(head, "lib.rs"), os.path.join(head, "main.rs"), head + ".rs"]
    decl = re.compile(rf"^\s*(pub(\([^)]*\))?\s+)?mod\s+{re.escape(stem)}\s*;")
    for parent in parents:
        parent = parent.replace("\\", "/")
        lines = lines_of(rev, parent)
        for i, line in enumerate(lines):
            if decl.search(line):
                k = i - 1
                while k >= 0 and lines[k].lstrip().startswith(("#[", "///", "//")):
                    if cfg.search(lines[k]):
                        return True
                    k -= 1
                return module_gated(rev, parent, cfg, seen + 1)
    return False


def changed_lines(base, head, path):
    """(new line numbers, old line numbers), 0-based, of the change."""
    new, old = [], []
    diff = git("diff", "-U0", f"{base}...{head}", "--", path)
    for m in re.finditer(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", diff, re.M):
        o, oc, n, nc = int(m[1]), int(m[2] or 1), int(m[3]), int(m[4] or 1)
        old += range(o - 1, o - 1 + oc)
        new += range(n - 1, n - 1 + nc)
    return new, old


def main():
    platform, base, head = sys.argv[1:4]
    cfg = re.compile(PATTERNS[platform])
    merge_base = git("merge-base", base, head).strip() or base
    files = [f for f in git("diff", "--name-only", f"{base}...{head}", "--", "*.rs").split("\n") if f]
    for path in files:
        new_lines, old_lines = lines_of(head, path), lines_of(merge_base, path)
        if module_gated(head, path, cfg) or (old_lines and module_gated(merge_base, path, cfg)):
            print(path)
            return 0
        new, old = changed_lines(base, head, path)
        if any(i < len(new_lines) and gated(new_lines, i, cfg) for i in new):
            print(path)
            return 0
        if any(i < len(old_lines) and gated(old_lines, i, cfg) for i in old):
            print(path)
            return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
