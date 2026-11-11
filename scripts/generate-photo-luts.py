"""Pack a creator's .cube look-up tables for the photo mode's LUT tab.

Input: a folder of .cube files (Adobe / Resolve "Cube LUT Specification"
1.0, 3D tables only), or a pack of the older 16-bit format (AURLUTS1) to
convert. Output: one pack file, read by
aurora-viewer/src/snapshot/lut_packs.rs (keep the two formats in step).

Each table keeps its size. Its colours are mapped from its domain to 0..1,
clamped there and stored as 8-bit values: the photo's look samples an 8-bit
texture of 0..1 (`Lut::rgba8`, round(v * 255)), so the look is the same
as with the full-precision table. Each channel is stored as its difference
from the previous colour's, then the table is zlib-compressed: a look's
colours change slowly, so a 33^3 table takes a few tens of kilobytes. The
table's name is its file name without the extension, less a `<prefix>` the
creator put before every name (the pack's heading in the viewer already
names them). The tables are stored in the order of their names, ignoring
case.

    python scripts/generate-photo-luts.py <folder or old .bin> <output.bin> [--prefix "Czar - "] [--skip <name>]...

Format, little-endian:
    b"AURLUTS2", u16 table count, then per table:
        u8 name length, the name in UTF-8, u8 side n, u32 length of the
        compressed colours, then the colours zlib-compressed: n^3 x
        (u8 red, u8 green, u8 blue), 255 = 1.0, red fastest, then green,
        then blue (the .cube order), each byte less the same channel of
        the previous colour (modulo 256, 0 before the first colour)
"""
import argparse
import struct
import zlib
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


def read_old_pack(raw):
    """The tables of an AURLUTS1 pack (16-bit colours), as 0..1 floats."""
    if raw[:8] != b"AURLUTS1":
        raise ValueError("not an AURLUTS1 pack")
    (count,) = struct.unpack_from("<H", raw, 8)
    p = 10
    tables = []
    for _ in range(count):
        n = raw[p]
        name = raw[p + 1 : p + 1 + n].decode("utf-8")
        p += 1 + n
        size = raw[p]
        p += 1
        vals = struct.unpack_from(f"<{size ** 3 * 3}H", raw, p)
        p += size**3 * 6
        data = [[v / 65535 for v in vals[i : i + 3]] for i in range(0, len(vals), 3)]
        tables.append((name, size, data))
    if p != len(raw):
        raise ValueError("bytes after the last table")
    return tables


def encode(data):
    """A table's colours: 8-bit, each channel less the previous colour's, compressed."""
    out = bytearray()
    prev = [0, 0, 0]
    for c in data:
        for k in range(3):
            v = round(min(max(c[k], 0.0), 1.0) * 255)
            out.append((v - prev[k]) & 255)
            prev[k] = v
    return zlib.compress(bytes(out), 9)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("source", type=Path, help="a folder of .cube files, or an AURLUTS1 pack")
    ap.add_argument("output", type=Path)
    ap.add_argument("--prefix", default="")
    ap.add_argument("--skip", action="append", default=[], help="a table name to leave out")
    args = ap.parse_args()

    tables = []
    if args.source.is_file():
        for name, size, data in read_old_pack(args.source.read_bytes()):
            if name in args.skip:
                print(f"skipped {name}")
                continue
            tables.append((name, size, data))
    else:
        for path in args.source.iterdir():
            if path.suffix.lower() != ".cube":
                continue
            name = path.stem
            if args.prefix and name.startswith(args.prefix):
                name = name[len(args.prefix) :]
            if name in args.skip or path.stem in args.skip:
                print(f"skipped {path.name}")
                continue
            size, data = parse_cube(path.read_text(encoding="utf-8", errors="replace"))
            tables.append((name, size, data))
    tables.sort(key=lambda t: t[0].lower())

    out = bytearray(b"AURLUTS2")
    out += struct.pack("<H", len(tables))
    for name, size, data in tables:
        raw = name.encode("utf-8")
        colours = encode(data)
        out += struct.pack("<B", len(raw)) + raw + struct.pack("<BI", size, len(colours)) + colours
        print(f"{name}: {size}^3, {len(colours)} bytes")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(bytes(out))
    print(f"{len(tables)} tables, {len(out)} bytes -> {args.output}")


if __name__ == "__main__":
    main()
