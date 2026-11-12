# Synthetic audio fixture

`abrupt-tone.ogg` is an original 120 ms, 440 Hz mono cosine at 44.1 kHz,
amplitude 0.6, cut without an envelope. It contains no grid or user assets.
It reproduces nonzero decoded clip boundaries and Vorbis edge ringing
for issue #1653.

Generate with FFmpeg (libvorbis):

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.6*cos(2*PI*440*t):s=44100:d=0.12' -c:a libvorbis -q:a 5 -map_metadata -1 abrupt-tone.ogg
```
