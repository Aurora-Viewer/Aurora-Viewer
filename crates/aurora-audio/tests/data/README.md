# Synthetic audio fixtures

Original synthetic tones generated with FFmpeg (libvorbis). They contain no
grid or user assets.

`abrupt-tone.ogg` is a 120 ms, 440 Hz mono cosine at 44.1 kHz, amplitude
0.6, cut without an envelope. It reproduces nonzero decoded clip boundaries
and Vorbis edge ringing for issue #1653.

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.6*cos(2*PI*440*t):s=44100:d=0.12' -c:a libvorbis -q:a 5 -map_metadata -1 abrupt-tone.ogg
```

`stereo-48k.ogg` and `stereo-128k.ogg` are 100 ms of a 440 Hz (left) and
660 Hz (right) sine, amplitude 0.5, at 48 kHz and 128 kHz; `three-channels.ogg`
is 50 ms of 440, 550 and 660 Hz sines at 44.1 kHz. They are sound assets in
formats the upload never makes, read as the Legacy Viewer reads them (issue
#3766).

```sh
ffmpeg -f lavfi -i 'aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*660*t):s=48000:d=0.1' -c:a libvorbis -q:a 0 -map_metadata -1 stereo-48k.ogg
ffmpeg -f lavfi -i 'aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*660*t):s=128000:d=0.1' -c:a libvorbis -q:a 0 -map_metadata -1 stereo-128k.ogg
ffmpeg -f lavfi -i 'aevalsrc=0.5*sin(2*PI*440*t)|0.5*sin(2*PI*550*t)|0.5*sin(2*PI*660*t):s=44100:d=0.05' -c:a libvorbis -q:a 0 -map_metadata -1 three-channels.ogg
```
