"""Build the 3D sound's head-related impulse responses from MIT KEMAR.

Input: the folder of the MIT Media Lab's diffuse-field equalized KEMAR set
(`diffuse.zip` of https://sound.media.mit.edu/resources/KEMAR.html, by Bill
Gardner and Keith Martin, Copyright 1994 MIT Media Lab, free to use with
credit). Output: assets/hrtf/kemar.bin, read by aurora-audio/src/hrtf.rs
(keep the two formats in step).

Each response (128 taps at 44.1 kHz, azimuths 0 to 180 degrees, the other
side being its mirror image) becomes:
- a minimum-phase filter at 48 kHz (the mixer's rate) of TAPS taps, built
  from its magnitude, so that neighbouring directions blend without the comb
  filtering of responses that start at different times; the magnitude is
  held below LOW_HOLD Hz (the 128-tap measurement has no bass, and the head
  lets the low notes through to both ears alike) and faded out above 20 kHz;
- the delay of each ear, measured by cross-correlating the response with
  its minimum-phase version; only the interaural time difference is kept
  (the earlier ear of each direction at 0).

The filters are scaled so that a sound heard from every direction at once
keeps its level: their mean energy over the sphere is 1 per ear.

Requires numpy (FFTs). The output is deterministic for a given input.

    python scripts/generate-hrtf.py <diffuse folder> assets/hrtf/kemar.bin

Format, little-endian:
    b"AURHRTF1", u32 sample rate, u16 taps, u16 rings, then per ring
    (elevation -40 to 90 degrees, in order):
        i16 elevation, u16 azimuths over the full circle, u16 stored
    then per stored direction (rings in order, azimuth 0 upwards, up to
    180 degrees; azimuth 90 is on the right):
        f32 delay of the left ear, f32 of the right ear (samples)
        taps x f32 left filter, taps x f32 right filter
"""
import argparse
import struct
import wave
from pathlib import Path

import numpy as np

SOURCE_RATE = 44_100
RATE = 48_000
TAPS = 64
FFT = 1024
LOW_HOLD = 300.0
HIGH_FADE = (20_000.0, 23_500.0)
UPSAMPLE = 16
# (elevation, azimuths over the full circle), MIT KEMAR Table 1
RINGS = [(-40, 56), (-30, 60), (-20, 72), (-10, 72), (0, 72), (10, 72), (20, 72),
         (30, 60), (40, 56), (50, 45), (60, 36), (70, 24), (80, 12), (90, 1)]


def load(folder, elev, az):
    path = folder / f"elev{elev}" / f"H{elev}e{az:03d}a.wav"
    with wave.open(str(path)) as w:
        if (w.getnchannels(), w.getsampwidth(), w.getframerate()) != (2, 2, SOURCE_RATE):
            raise SystemExit(f"{path}: not 16-bit stereo at 44.1 kHz")
        data = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2")
    return data.astype(np.float64).reshape(-1, 2).T / 32768.0


def resample(h):
    """Band-limited 44.1 -> 48 kHz (441 -> 480 through the spectrum)."""
    x = np.zeros(441 * 4)
    x[: len(h)] = h
    spec = np.fft.rfft(x)
    out = np.zeros(480 * 4 // 2 + 1, dtype=complex)
    out[: len(spec)] = spec
    return np.fft.irfft(out, 480 * 4) * (480 / 441)


def shaped_magnitude(h):
    freqs = np.fft.rfftfreq(FFT, 1.0 / RATE)
    mag = np.abs(np.fft.rfft(h, FFT))
    hold = np.searchsorted(freqs, LOW_HOLD)
    mag[:hold] = mag[hold]
    lo, hi = HIGH_FADE
    t = np.clip((freqs - lo) / (hi - lo), 0.0, 1.0)
    mag *= 0.5 + 0.5 * np.cos(np.pi * t)
    return np.maximum(mag, 1e-5)


def minimum_phase(mag):
    """Real cepstrum, folded onto the positive quefrencies."""
    cep = np.fft.irfft(np.log(mag), FFT)
    fold = np.zeros(FFT)
    fold[0] = cep[0]
    fold[1 : FFT // 2] = 2.0 * cep[1 : FFT // 2]
    fold[FFT // 2] = cep[FFT // 2]
    return np.fft.irfft(np.exp(np.fft.rfft(fold)), FFT)


def delay_of(h, mp):
    """Lag of h behind mp, in samples (to a sixteenth)."""
    n = FFT * UPSAMPLE
    cross = np.fft.rfft(h, FFT) * np.conj(np.fft.rfft(mp, FFT))
    full = np.zeros(n // 2 + 1, dtype=complex)
    full[: len(cross)] = cross
    corr = np.fft.irfft(full, n)
    return int(np.argmax(corr[: n // 2])) / UPSAMPLE


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("diffuse", type=Path, help="folder of the diffuse set (elev*/H*e*a.wav)")
    parser.add_argument("out", type=Path, help="assets/hrtf/kemar.bin")
    args = parser.parse_args()

    rings = []
    entries = []  # (ring, index in the ring, delays[2], filters[2])
    for ring, (elev, count) in enumerate(RINGS):
        stored = count // 2 + 1
        rings.append((elev, count, stored))
        for i in range(stored):
            pair = load(args.diffuse, elev, int(round(i * 360.0 / count)))
            delays, filters = [], []
            for ear in range(2):
                h = resample(pair[ear])
                mp = minimum_phase(shaped_magnitude(h))
                delays.append(delay_of(h, mp))
                filters.append(mp)
            entries.append((ring, i, delays, filters))

    # mean energy over the sphere: each direction weighs its share of its
    # ring's band of solid angle
    total, energy = 0.0, 0.0
    for ring, i, _, filters in entries:
        elev, count, _ = rings[ring]
        lo = -90.0 if elev == -40 else elev - 5.0
        hi = min(90.0, elev + 5.0)
        w = (np.sin(np.radians(hi)) - np.sin(np.radians(lo))) / count
        # azimuths 0 and 180 stand for one direction, the others for two
        both = 1 if i == 0 or i * 2 == count else 2
        for f in filters:
            energy += w * both * float(np.sum(f[:TAPS] ** 2))
            total += w * both
    scale = 1.0 / np.sqrt(energy / total)

    kept = min(float(np.sum(f[:TAPS] ** 2) / np.sum(f**2)) for _, _, _, fs in entries for f in fs)
    fade = np.ones(TAPS)
    fade[-8:] = 0.5 + 0.5 * np.cos(np.pi * (np.arange(8) + 1) / 9)
    out = bytearray(b"AURHRTF1")
    out += struct.pack("<IHH", RATE, TAPS, len(rings))
    for elev, count, stored in rings:
        out += struct.pack("<hHH", elev, count, stored)
    for _, _, delays, filters in entries:
        first = min(delays)
        out += struct.pack("<ff", delays[0] - first, delays[1] - first)
        for f in filters:
            out += struct.pack(f"<{TAPS}f", *(f[:TAPS] * fade * scale))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_bytes(bytes(out))
    widest = max(max(d) - min(d) for _, _, d, _ in entries)
    print(f"{len(entries)} directions, {TAPS} taps, {len(out)} bytes; interaural delays up to "
          f"{widest:.2f} samples; energy kept at worst {kept:.5f}; scale {scale:.4f}")


if __name__ == "__main__":
    main()
