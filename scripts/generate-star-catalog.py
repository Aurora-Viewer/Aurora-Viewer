"""Build the night sky's star table and Milky Way map from Tycho-2.

Inputs:
  - the Tycho-2 catalogue (Hog et al. 2000; BSD-2-Clause, see
    assets/stars/LICENSE-TYCHO2.md): the folder holding catalog.dat and
    suppl_1.dat (plain or .gz), from https://archive.eso.org/ASTROM/TYC-2/data/
    or CDS (I/259);
  - the Hipparcos main catalogue (ESA 1997, ESA SP-1200) as HEASARC's
    heasarc_hipparcos.tdat.gz, for the few stars too bright for Tycho-2's
    photometry (below).

Outputs, read by aurora-render/src/stars.rs (keep the formats in step):
  - assets/stars/stars.bin: the stars visible to the naked eye under a dark
    sky (V <= 6.5), brightest first;
  - assets/stars/milky_way.bin: the Milky Way, from the counts of the stars
    fainter than the naked eye, on a grid of galactic longitude and
    latitude.

Tycho-2's photometry saturates on the brightest stars: its main catalogue
has none brighter than VT 1.9, and its supplement gives the brightest only
a Hipparcos Hp magnitude, without colour (Sirius at -1.09 where V is -1.46,
Arcturus and Capella without their colour). For a star brighter than V 2.0
or without a colour index, its Hipparcos number takes V and B-V from the
Hipparcos main catalogue instead.

Standard library only; the output is deterministic for given inputs.

    python scripts/generate-star-catalog.py <tycho2 folder> heasarc_hipparcos.tdat.gz assets/stars

stars.bin, little-endian:
    b"AURSTAR1", u32 count, then per star:
    u16 right ascension  (0..65535 for 0..360 degrees, J2000)
    i16 declination      (-32767..32767 for -90..90 degrees)
    i16 magnitude x 100
    i16 B-V colour index x 1000 (32767: unknown)

milky_way.bin, little-endian:
    b"AURMWAY2", u32 width, u32 height, f32 latitude reach (degrees),
    u32 colour width, u32 colour height, u32 length, then `length` bytes of
    zlib holding:
      - width x height u16 texels, row by row from latitude -reach to
        +reach, column x covering galactic longitude -180 + 360 x / width to
        the next column: bits 0..7 the glow (sqrt of the faint stars' light,
        255 the brightest texel), bits 8..15 the grain (stars of V 6.5..9.5
        per texel, x 16);
      - colour width x colour height u8 over the same area: B-V colour index
        of the faint stars, x 100 (0..2.55), smooth enough for a coarse grid.
"""
import argparse
import gzip
import math
import struct
import zlib
from pathlib import Path

MAX_MAG = 6.5
NO_COLOUR = 32767
# Tycho-2's photometry saturates above this brightness
SATURATED = 2.0
# Milky Way grid: 0.176 degrees per texel, latitudes within REACH
WIDTH, HEIGHT, REACH = 2048, 512, 45.0
# the colour grid: one value per COLOUR_STEP x COLOUR_STEP texels
COLOUR_STEP = 8
# the glow's stars: fainter than this (the grain's are between MAX_MAG and it)
FAINT = 9.5
# Gaussian smoothing of the counts, in texels: a few stars per texel
GLOW_SIGMA = 0.9
GRAIN_SIGMA = 1.0
# the light beyond Tycho-2's reach (deep): the thin plane at every longitude
# (latitude half-width in degrees, gain), the inner disk (longitude and
# latitude half-widths, gain) and the bulge (half-width, gain)
DEEP_PLANE_B, DEEP_PLANE = 5.0, 2.0
DEEP_DISK_L, DEEP_DISK_B, DEEP_DISK = 50.0, 6.0, 2.0
DEEP_BULGE, DEEP_BULGE_GAIN = 9.0, 3.0
# the faint stars' count left out as the floor of the whole sky: this
# fraction of the count far from the plane (its texels' median)
FLOOR = 1.6
# J2000 equatorial to galactic (IAU 1958, Hipparcos values); env.wgsl
# milky_way uses the same rows
GALACTIC = (
    (-0.0548756, -0.8734371, -0.4838350),
    (0.4941094, -0.4448296, 0.7469822),
    (-0.8676661, -0.1980764, 0.4559838),
)


def number(field):
    field = field.strip()
    return float(field) if field else None


def johnson(bt, vt):
    """V and B-V from Tycho BT and VT (Hipparcos and Tycho Catalogues,
    ESA SP-1200, vol. 1, sect. 1.3)."""
    if bt is not None and vt is not None:
        return vt - 0.090 * (bt - vt), 0.850 * (bt - vt)
    return (vt if vt is not None else bt), None


def lines(folder, name):
    path = folder / name
    if not path.exists():
        return gzip.open(folder / (name + ".gz"), "rt", encoding="ascii")
    return open(path, encoding="ascii")


def tycho2(folder):
    """Every star of Tycho-2 and its first supplement: (V, B-V, RA, Dec at
    J2000 in degrees, HIP number, identifier)."""
    for line in lines(folder, "catalog.dat"):
        v, bv = johnson(number(line[110:116]), number(line[123:129]))
        ra, dec = number(line[15:27]), number(line[28:40])
        if ra is None:
            # no mean position: the observed one (epoch about 1991)
            ra, dec = number(line[152:164]), number(line[165:177])
        hip = line[142:148].strip()
        yield v, bv, ra, dec, int(hip) if hip else None, line[0:12]
    for line in lines(folder, "suppl_1.dat"):
        bt, vt = number(line[83:89]), number(line[96:102])
        if line[81] == "H":
            # Hp instead of VT, no BT
            v, bv = vt, None
        else:
            v, bv = johnson(bt, vt)
        ra, dec = number(line[15:27]), number(line[28:40])
        pm_ra, pm_dec = number(line[41:48]), number(line[49:56])
        if pm_ra is not None and pm_dec is not None:
            # J1991.25 to J2000
            years = 8.75
            dec += pm_dec * years / 3.6e6
            ra += pm_ra * years / 3.6e6 / max(math.cos(math.radians(dec)), 1e-6)
        hip = line[115:121].strip()
        yield v, bv, ra, dec, int(hip) if hip else None, line[0:12]


def hipparcos(path):
    """HIP number -> (V, B-V) of the Hipparcos main catalogue."""
    fields, out = None, {}
    with gzip.open(path, "rt", encoding="latin-1") as f:
        data = False
        for line in f:
            if line.startswith("line[1]"):
                fields = line.split("=", 1)[1].split()
            elif line.startswith("<DATA>"):
                data = True
            elif line.startswith("<END>"):
                break
            elif data and fields:
                row = dict(zip(fields, line.rstrip("\n").split("|")))
                v = number(row.get("vmag", ""))
                if v is not None and row.get("hip_number", "").strip():
                    out[int(row["hip_number"])] = (v, number(row.get("bv_color", "")))
    return out


def galactic(ra, dec):
    a, d = math.radians(ra), math.radians(dec)
    q = (math.cos(d) * math.cos(a), math.cos(d) * math.sin(a), math.sin(d))
    g = [sum(r[i] * q[i] for i in range(3)) for r in GALACTIC]
    return math.degrees(math.atan2(g[1], g[0])), math.degrees(math.asin(max(-1.0, min(1.0, g[2]))))


def blur(grid, sigma):
    """Separable Gaussian; longitude wraps, latitude clamps."""
    r = int(math.ceil(3 * sigma))
    k = [math.exp(-0.5 * (i / sigma) ** 2) for i in range(-r, r + 1)]
    s = sum(k)
    k = [w / s for w in k]
    rows = []
    for y in range(HEIGHT):
        row = grid[y * WIDTH:(y + 1) * WIDTH]
        rows.append([sum(k[j] * row[(x + j - r) % WIDTH] for j in range(len(k))) for x in range(WIDTH)])
    out = [0.0] * (WIDTH * HEIGHT)
    for y in range(HEIGHT):
        for x in range(WIDTH):
            out[y * WIDTH + x] = sum(k[j] * rows[min(HEIGHT - 1, max(0, y + j - r))][x] for j in range(len(k)))
    return out


def deep(i):
    """The light of the stars too faint for Tycho-2 relative to its counts.
    Tycho-2 (V 11.5) counts stars within a few thousand light years, while
    the Milky Way's light comes mostly from farther: towards the centre
    (Sagittarius, Scutum) its counts stop at the dust, where the eye sees
    the brightest star clouds of the sky. A smooth weight of the inner disk
    and the bulge puts that light back, and so does a thin plane at every
    longitude: the far stars crowd close to it, where Tycho-2's nearby ones
    spread over the whole sky. The clouds and dark lanes stay those of the
    counts."""
    l = ((i % WIDTH) + 0.5) / WIDTH * 360.0 - 180.0
    b = ((i // WIDTH) + 0.5) / HEIGHT * 2.0 * REACH - REACH
    disk = math.exp(-((l / DEEP_DISK_L) ** 2) - (b / DEEP_DISK_B) ** 2)
    bulge = math.exp(-((l / DEEP_BULGE) ** 2) - (b / DEEP_BULGE) ** 2)
    plane = math.exp(-((b / DEEP_PLANE_B) ** 2))
    return 1.0 + DEEP_PLANE * plane + DEEP_DISK * disk + DEEP_BULGE_GAIN * bulge


def write_stars(stars, path):
    stars.sort(key=lambda s: (s[0], s[4]))
    out = bytearray(b"AURSTAR1")
    out += struct.pack("<I", len(stars))
    for mag, bv, ra_deg, dec_deg, _id in stars:
        ra = round(ra_deg % 360.0 / 360.0 * 65536.0) % 65536
        dec = max(-32767, min(32767, round(dec_deg / 90.0 * 32767.0)))
        c = NO_COLOUR if bv is None else max(-32767, min(32766, round(bv * 1000.0)))
        out += struct.pack("<Hhhh", ra, dec, round(mag * 100.0), c)
    path.write_bytes(bytes(out))
    return len(out)


def write_map(faint, grain, colour, weight, path):
    glow = blur(faint, GLOW_SIGMA)
    grain = blur(grain, GRAIN_SIGMA)
    colour = blur(colour, COLOUR_STEP)
    weight = blur(weight, COLOUR_STEP)
    # the faint stars' count far from the plane is a floor spread over the
    # whole sky: the Milky Way is what rises above it
    edge = [glow[y * WIDTH + x] for y in list(range(0, 24)) + list(range(HEIGHT - 24, HEIGHT)) for x in range(WIDTH)]
    edge.sort()
    floor = edge[len(edge) // 2] * FLOOR
    light = [max(0.0, g - floor) * deep(i) for i, g in enumerate(glow)]
    top = sorted(light)[int(len(light) * 0.9995)]
    texels = bytearray()
    for i in range(WIDTH * HEIGHT):
        y = i // WIDTH
        b = abs((y + 0.5) / HEIGHT * 2.0 - 1.0)
        # nothing at the grid's latitude edge, so the sky has no seam there
        fade = min(1.0, (1.0 - b) / 0.15)
        g = min(255, round(math.sqrt(min(1.0, light[i] * fade / top)) * 255.0))
        n = min(255, round(grain[i] * fade * 16.0))
        texels += struct.pack("<H", g | n << 8)
    cw, ch = WIDTH // COLOUR_STEP, HEIGHT // COLOUR_STEP
    for cy in range(ch):
        for cx in range(cw):
            i = (cy * COLOUR_STEP + COLOUR_STEP // 2) * WIDTH + cx * COLOUR_STEP + COLOUR_STEP // 2
            bv = colour[i] / weight[i] if weight[i] > 1e-6 else 0.6
            texels.append(max(0, min(255, round(bv * 100.0))))
    packed = zlib.compress(bytes(texels), 9)
    path.write_bytes(b"AURMWAY2" + struct.pack("<IIfIII", WIDTH, HEIGHT, REACH, cw, ch, len(packed)) + packed)
    return len(packed) + 32


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("tycho2", type=Path, help="folder with catalog.dat and suppl_1.dat")
    parser.add_argument("hipparcos", type=Path, help="heasarc_hipparcos.tdat.gz")
    parser.add_argument("out", type=Path, help="assets/stars")
    args = parser.parse_args()

    hip = hipparcos(args.hipparcos)
    faint = [0.0] * (WIDTH * HEIGHT)
    grain = [0.0] * (WIDTH * HEIGHT)
    colour = [0.0] * (WIDTH * HEIGHT)
    weight = [0.0] * (WIDTH * HEIGHT)
    # the stars near the naked-eye limit wait for their Hipparcos fill
    near = []
    for v, bv, ra, dec, hip_number, ident in tycho2(args.tycho2):
        if v is None or ra is None:
            continue
        if v <= MAX_MAG + 0.5:
            near.append((v, bv, ra, dec, hip_number, ident))
            continue
        add_to_map(v, bv, ra, dec, faint, grain, colour, weight)
    stars = []
    for v, bv, ra, dec, ident in fill(near, hip):
        if v <= MAX_MAG:
            stars.append((v, bv, ra, dec, ident))
        else:
            add_to_map(v, bv, ra, dec, faint, grain, colour, weight)

    args.out.mkdir(parents=True, exist_ok=True)
    size = write_stars(stars, args.out / "stars.bin")
    print(f"{len(stars)} stars (V <= {MAX_MAG}) -> stars.bin ({size} bytes)")
    size = write_map(faint, grain, colour, weight, args.out / "milky_way.bin")
    print(f"Milky Way {WIDTH} x {HEIGHT} -> milky_way.bin ({size} bytes)")


def fill(near, hip):
    """The stars near the naked-eye limit, with the Hipparcos V and B-V for
    a star Tycho-2 cannot measure (brighter than SATURATED, or without a
    colour). Hipparcos gives a system's combined light: the system's other
    Tycho-2 components, a few arcseconds away, are then left out."""
    brightest = {}
    for s in near:
        if s[4] is not None and (s[4] not in brightest or s[0] < brightest[s[4]][0]):
            brightest[s[4]] = s
    for s in near:
        v, bv, ra, dec, number_, ident = s
        main = brightest.get(number_)
        if main is not None and number_ in hip and (main[0] < SATURATED or main[1] is None):
            if s is not main:
                continue
            v, bv = hip[number_]
        yield v, bv, ra, dec, ident


def add_to_map(v, bv, ra, dec, faint, grain, colour, weight):
    """A star fainter than the naked eye into the map: the glow's count and
    colour, or the grain's count."""
    l, b = galactic(ra, dec)
    if abs(b) >= REACH:
        return
    x = min(WIDTH - 1, int((l + 180.0) / 360.0 * WIDTH))
    y = min(HEIGHT - 1, int((b + REACH) / (2.0 * REACH) * HEIGHT))
    i = y * WIDTH + x
    if v > FAINT:
        faint[i] += 1.0
        if bv is not None and -0.5 < bv < 3.0:
            colour[i] += bv
            weight[i] += 1.0
    else:
        grain[i] += 1.0


if __name__ == "__main__":
    main()
