# Head-related impulse responses

`kemar.bin` holds the filters of the 3D sound for headphones (Preferences ›
Sound, "3D sound for headphones (HRTF)"): for 368 directions around the
head, from 40 degrees below the ears to straight above, a pair of 64-tap
minimum-phase filters at 48 kHz and the delay of each ear. The mixer renders
world sounds through them (`crates/aurora-audio/src/hrtf.rs`); the other
half of the sphere is their mirror image.

They are derived from the **MIT Media Lab's KEMAR HRTF measurements** by
Bill Gardner and Keith Martin, the diffuse-field equalized set
(`diffuse.zip`), https://sound.media.mit.edu/resources/KEMAR.html:

> This data is Copyright 1994 by the MIT Media Laboratory. It is provided
> free with no restrictions on use, provided the authors are cited when the
> data is used in any research or commercial application.

Bill Gardner and Keith Martin, "HRTF Measurements of a KEMAR Dummy-Head
Microphone", MIT Media Lab Perceptual Computing Technical Report #280, 1994.

Changes: each 44.1 kHz response is resampled to 48 kHz and replaced by the
minimum-phase filter of its magnitude (held below 300 Hz, faded out above
20 kHz), cut to 64 taps; the delay between its two ears is measured and kept
apart; all the filters are scaled together so that their mean energy over the
sphere is 1 per ear.

To rebuild it (Python with numpy):

```powershell
python scripts/generate-hrtf.py <folder of diffuse.zip, unpacked> assets/hrtf/kemar.bin
```
