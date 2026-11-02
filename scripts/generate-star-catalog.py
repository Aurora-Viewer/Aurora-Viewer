"""Build the night sky's star table from the HYG database.

Input: the HYG database CSV (gzip), https://codeberg.org/astronexus/hyg,
licensed CC BY-SA 4.0. Output: assets/stars/stars.bin, read by
aurora-render/src/stars.rs (keep the two formats in step).

Keeps the stars visible to the naked eye under a dark sky (visual
magnitude <= 6.5), brightest first, without the Sun. Standard library only;
the output is deterministic for a given input.

    python scripts/generate-star-catalog.py hyg_v44.csv.gz assets/stars/stars.bin

Format, little-endian:
    b"AURSTAR1", u32 count, then per star:
    u16 right ascension  (0..65535 for 0..360 degrees)
    i16 declination      (-32767..32767 for -90..90 degrees)
    i16 magnitude x 100
    i16 B-V colour index x 1000 (32767: unknown)
"""
import argparse
import csv
import gzip
import struct
from pathlib import Path

MAX_MAG = 6.5
NO_COLOUR = 32767


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("hyg", type=Path, help="hyg_vNN.csv.gz")
    parser.add_argument("out", type=Path, help="assets/stars/stars.bin")
    args = parser.parse_args()

    stars = []
    with gzip.open(args.hyg, "rt", encoding="utf-8", newline="") as f:
        for row in csv.DictReader(f):
            if row["proper"] == "Sol" or not row["mag"]:
                continue
            mag = float(row["mag"])
            if mag > MAX_MAG:
                continue
            ra_deg = float(row["ra"]) * 15.0  # hours to degrees
            dec_deg = float(row["dec"])
            ci = row["ci"].strip()
            stars.append((mag, int(row["id"]), ra_deg, dec_deg, float(ci) if ci else None))

    stars.sort(key=lambda s: (s[0], s[1]))
    out = bytearray(b"AURSTAR1")
    out += struct.pack("<I", len(stars))
    for mag, _id, ra_deg, dec_deg, ci in stars:
        ra = round(ra_deg % 360.0 / 360.0 * 65536.0) % 65536
        dec = max(-32767, min(32767, round(dec_deg / 90.0 * 32767.0)))
        m = round(mag * 100.0)
        c = NO_COLOUR if ci is None else max(-32767, min(32766, round(ci * 1000.0)))
        out += struct.pack("<Hhhh", ra, dec, m, c)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(bytes(out))
    print(f"{len(stars)} stars (magnitude <= {MAX_MAG}) -> {args.out} ({len(out)} bytes)")


if __name__ == "__main__":
    main()
