"""Pack a creator's .cube look-up tables for the photo mode's LUT tab.

Input: a folder of .cube files (Adobe / Resolve "Cube LUT Specification"
1.0, 3D tables only). Output: one pack file, read by
aurora-viewer/src/snapshot/lut_packs.rs (keep the two formats in step).

Each table keeps its size and its colours, mapped from its domain to 0..1
and clamped there (the photo's look samples an 8-bit texture of 0..1, so
nothing visible is lost), as 16-bit fixed point. The table's name is its
file name without the extension, less a `<prefix>` the creator put before
every name (the pack's heading in the viewer already names them). The
tables are stored in the order of their names, ignoring case.

    python scripts/generate-photo-luts.py <folder> <output.bin> [--prefix "Czar - "] [--skip <name>]...

Format, little-endian:
    b"AURLUTS1", u16 table count, then per table:
        u8 name length, the name in UTF-8, u8 side n,
        n^3 x (u16 red, u16 green, u16 blue), 65535 = 1.0, red fastest,
        then green, then blue (the .cube order)
"""
import argparse
import struct
from pathlib import Path


def parse_cube(text):
    size = None
    lo = [0.0, 0.0, 0.0]
    hi = [1.0, 1.0, 1.0]
    data = []
    for line in text.splitlines():
        words = line.split()
        if not words or words[0].startswith("#"):
            continue
        key = words[0]
        if key in ("TITLE", "LUT_1D_INPUT_RANGE", "LUT_3D_INPUT_RANGE"):
            continue
        if key == "LUT_1D_SIZE":
            raise ValueError("a 1D table is not supported")
        if key == "LUT_3D_SIZE":
            size = int(words[1])
            continue
        if key == "DOMAIN_MIN":
            lo = [float(w) for w in words[1:4]]
            continue
        if key == "DOMAIN_MAX":
            hi = [float(w) for w in words[1:4]]
            continue
        v = [float(w) for w in words]
        if len(v) != 3:
            raise ValueError(f"not a colour: {line!r}")
        data.append([(v[k] - lo[k]) / max(hi[k] - lo[k], 1e-6) for k in range(3)])
    if size is None or not 2 <= size <= 128:
        raise ValueError(f"bad or missing LUT_3D_SIZE: {size}")
    if len(data) != size**3:
        raise ValueError(f"{len(data)} colours for {size ** 3}")
    return size, data


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("folder", type=Path)
    ap.add_argument("output", type=Path)
    ap.add_argument("--prefix", default="")
    ap.add_argument("--skip", action="append", default=[], help="a table name to leave out")
    args = ap.parse_args()

    tables = []
    for path in args.folder.iterdir():
        if path.suffix.lower() != ".cube":
            continue
        name = path.stem
        if args.prefix and name.startswith(args.prefix):
            name = name[len(args.prefix):]
        if name in args.skip or path.stem in args.skip:
            print(f"skipped {path.name}")
            continue
        size, data = parse_cube(path.read_text(encoding="utf-8", errors="replace"))
        tables.append((name, size, data))
    tables.sort(key=lambda t: t[0].lower())

    out = bytearray(b"AURLUTS1")
    out += struct.pack("<H", len(tables))
    for name, size, data in tables:
        raw = name.encode("utf-8")
        out += struct.pack("<B", len(raw)) + raw + struct.pack("<B", size)
        for c in data:
            out += struct.pack("<3H", *(round(min(max(v, 0.0), 1.0) * 65535) for v in c))
        print(f"{name}: {size}^3")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(bytes(out))
    print(f"{len(tables)} tables, {len(out)} bytes -> {args.output}")


if __name__ == "__main__":
    main()
