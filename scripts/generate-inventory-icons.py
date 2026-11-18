"""Build the inventory's system folder icons from Tabler icons.

Each system folder icon is Tabler's `folder` (closed) or `folder-open`
(open) with a category's Tabler symbol cut out of the folder's body, so the
folder keeps one colour (the inventory tints it) and the symbol reads as a
hole. Nothing is drawn here: the shapes are Tabler's (MIT,
assets/tabler-icons/LICENSE), placed and combined with an SVG mask. The
script also adds the hand-drawn shortcut corner of the existing link icons
(assets/tabler-icons/filled/folder-link.svg, #1610) to the closed folders
and to the Tabler icons listed in LINKS, for inventory links.

The output is committed (assets/icons/aurora/), so the viewer embeds plain
SVG files and a skin can replace any of them by name. Run it again after
changing the tables below, and keep them in step with
crates/aurora-viewer/src/ui/inventory/category.rs (its tests read the
`aurora-generated` comment of every file):

    python scripts/generate-inventory-icons.py
"""
import pathlib
import re
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
TABLER = ROOT / "assets" / "tabler-icons"
OUT = ROOT / "assets" / "icons" / "aurora"
SVG_NS = "http://www.w3.org/2000/svg"

# Folder file name -> Tabler symbol ("-filled" names come from filled/).
FOLDERS = {
    "animations": "run",
    "body-parts": "man-filled",
    "calling-cards": "id-filled",
    "clothing": "shirt-filled",
    "current-outfit": "hanger-2-filled",
    "favorites": "star-filled",
    "gestures": "hand-move",
    "inbox": "inbox",
    "landmarks": "map-pin-filled",
    "lost-and-found": "help-circle-filled",
    "marketplace": "building-store",
    "materials": "sphere",
    "meshes": "cube-3d-sphere",
    "notecards": "file-text-filled",
    "objects": "box",
    "outfits": "hanger",
    "photo-album": "library-photo",
    "scripts": "code",
    "settings": "sun-filled",
    "sounds": "file-music-filled",
    "textures": "photo-filled",
}

# Tabler icons of items that get a link variant here (the older ones were
# made by hand in assets/tabler-icons/*/*-link.svg).
LINKS = ["cloud-filled", "ripple", "sun-moon", "sphere"]

# The symbol's size against the folder (24 units), and where its centre
# goes: the middle of the folder's body below the tab (closed), the middle
# of the front flap (open). The open folder's flap is only about 10 units
# tall, so its symbol is a little smaller, clear of the gap above the flap.
SCALE = {"closed": 0.42, "open": 0.37}
CENTER = {"closed": (12.0, 13.0), "open": (13.4, 15.5)}
# Width of an outline symbol's cut, in the symbol's units before scaling
# (Tabler draws at 2: thicker, so the cut stays open at 16 px).
OUTLINE_CUT = 3.6


def tabler(name):
    """The file of a Tabler icon: `x-filled` is filled/x.svg, `x` outline/x.svg."""
    if name.endswith("-filled"):
        return TABLER / "filled" / (name[: -len("-filled")] + ".svg")
    return TABLER / "outline" / (name + ".svg")


def local(tag):
    return tag.rsplit("}", 1)[-1]


def shapes(path):
    """The drawn elements of a Tabler icon, as SVG text without namespaces
    (Tabler's invisible 24x24 frame path left out)."""
    root = ET.parse(path).getroot()
    out = []
    for el in root:
        if el.get("d", "").replace(" ", "") == "M0 0h24v24H0z".replace(" ", ""):
            continue
        out.append(serialize(el))
    return "\n".join(out)


def serialize(el):
    attrs = "".join(
        f' {local(k)}="{v}"' for k, v in el.attrib.items() if not k.startswith("{") and k not in ("id", "class")
    )
    children = "".join(serialize(c) for c in el)
    tag = local(el.tag)
    return f"<{tag}{attrs}>{children}</{tag}>" if children else f"<{tag}{attrs} />"


def shortcut():
    """The shortcut corner of the hand-made link icons (its black outline
    knocks the icon out around it; see icons.rs `rasterize_mask`)."""
    text = (TABLER / "filled" / "folder-link.svg").read_text(encoding="utf-8")
    match = re.search(r'<path\s+fill="#ffffff"[^>]*?d="([^"]+)"', text, re.S)
    if not match:
        raise SystemExit("folder-link.svg: shortcut layer not found")
    return (
        '<path fill="#ffffff" stroke="#202222" stroke-width="0.267044" stroke-linejoin="round" '
        f'd="{match.group(1)}" />'
    )


def header(comment, root_attrs):
    return (
        f"<!-- aurora-generated: {comment} (scripts/generate-inventory-icons.py; "
        "Tabler icons, MIT, assets/tabler-icons/LICENSE) -->\n"
        f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" {root_attrs}>\n'
    )


def folder(state, symbol, link):
    base = "folder" if state == "closed" else "folder-open"
    body = shapes(TABLER / "filled" / f"{base}.svg")
    cx, cy = CENTER[state]
    if symbol.endswith("-filled"):
        paint = 'fill="#000000" stroke="none"'
    else:
        paint = (
            f'fill="none" stroke="#000000" stroke-width="{OUTLINE_CUT}" '
            'stroke-linecap="round" stroke-linejoin="round"'
        )
    transform = f"translate({cx} {cy}) scale({SCALE[state]}) translate(-12 -12)"
    svg = header(f"{base}-filled + {symbol}", 'fill="currentColor"')
    svg += (
        '  <defs>\n'
        '    <mask id="cut" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">\n'
        '      <rect width="24" height="24" fill="#ffffff" />\n'
        f'      <g transform="{transform}" {paint}>\n'
        f"{indent(shapes(tabler(symbol)), 8)}\n"
        "      </g>\n"
        "    </mask>\n"
        "  </defs>\n"
        f'  <g mask="url(#cut)">\n{indent(body, 4)}\n  </g>\n'
    )
    if link:
        svg += f"  {shortcut()}\n"
    return svg + "</svg>\n"


def linked(name):
    root = ET.parse(tabler(name)).getroot()
    keep = ("fill", "stroke", "stroke-width", "stroke-linecap", "stroke-linejoin")
    attrs = " ".join(f'{k}="{root.get(k)}"' for k in keep if root.get(k) is not None)
    svg = header(f"{name} + shortcut", attrs)
    svg += f"{indent(shapes(tabler(name)), 2)}\n  {shortcut()}\n</svg>\n"
    return svg


def indent(text, n):
    return "\n".join(" " * n + line for line in text.splitlines())


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")


def main():
    for name, symbol in FOLDERS.items():
        if not tabler(symbol).exists():
            raise SystemExit(f"{name}: no Tabler icon {symbol}")
        write(OUT / "folders" / f"{name}-closed.svg", folder("closed", symbol, False))
        write(OUT / "folders" / f"{name}-open.svg", folder("open", symbol, False))
        write(OUT / "folders" / f"{name}-closed-link.svg", folder("closed", symbol, True))
    for name in LINKS:
        write(OUT / "links" / f"{name}-link.svg", linked(name))
    print(f"{len(FOLDERS) * 3} folder icons, {len(LINKS)} link icons in {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
