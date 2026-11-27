# NVIDIA DLSS (optional build feature)

A viewer built with the `dlss` feature (off by default:
`cargo build --features aurora-viewer/dlss`) can draw the world with NVIDIA
DLSS super resolution. The viewer speaks only the public interface of
NVIDIA Streamline, NVIDIA's DLSS integration layer, whose headers are MIT
licensed. The files behind it are NVIDIA's, under NVIDIA's own licence, so
neither the viewer nor its package contains them: you download them from
NVIDIA and accept that licence yourself, once. A build without the feature
has no DLSS choice.

- **Windows:** NVIDIA's own signed Streamline release.
- **Linux:** NVIDIA ships Streamline for Windows only. The setup builds
  [sl-ngx](https://github.com/Aurora-Viewer/sl-ngx), an MIT library that
  implements Streamline's interface for Vulkan on NVIDIA's NGX, against
  NVIDIA's DLSS SDK.

A package of such a build made with `scripts/package.ps1 -Dlss` holds the
setup script next to the viewer: `dlss-setup.sh` on Linux,
`dlss-setup.ps1` on Windows. From a source checkout, run them from the
`scripts` folder.

## Install

1. Run, in a terminal, from the viewer's folder:

   ```bash
   ./dlss-setup.sh
   ```

   It downloads the Linux files of NVIDIA's DLSS SDK from
   <https://github.com/NVIDIA/DLSS> (about 60 MB) and shows you NVIDIA's
   licence. Once you type `yes`, it downloads sl-ngx at a fixed commit,
   builds it against the SDK and installs. It needs `git`, `make`, `g++` and
   the Vulkan headers (Arch: `vulkan-headers`; Debian, Ubuntu:
   `libvulkan-dev`). Already have the SDK? `./dlss-setup.sh --sdk <its folder>`.
2. Start the viewer (restart it if it was running).
3. Preferences › Graphics › Anti-aliasing: choose **NVIDIA DLSS**.
   Its group below, **NVIDIA DLSS**, chooses the mode: how much smaller the
   world is drawn before DLSS rebuilds it at the window's size (DLAA keeps
   the window's size), and the model: Automatic lets NVIDIA pick one per
   mode (a driver update may change it); K, L or M pins one in every mode.
   DLSS no longer sharpens by itself: Preferences › Graphics › Sharpness
   (CAS) applies after it, as after TAA.

NVIDIA graphics cards (RTX) only. On another card, or when the driver does
not open DLSS, the choice stays greyed with the reason, and the log says why.
When NVIDIA's library refuses DLSS while the viewer runs, FSR draws, DLSS
stays chosen with its mode and model, and the reason shows under the list
with a **Try again** button (it is not asked again by itself until another
device opens). Changing the model makes only the DLSS feature again. The
second camera and the VR eyes draw DLSS too; when it fails there they draw
FSR, and Preferences shows the main view's state, plus a start of NVIDIA's
library refused for any of them. Each eye falls back on its own, so after
a failure the two eyes can differ (DLSS in one, FSR in the other) until
**Try again**. For VR the headset's
device is opened with what DLSS needs too; if NVIDIA's library cannot add
that, FSR draws while the headset is in use and Preferences says so. DLSS in
VR was run on Windows by a contributor (Quest 3, Meta's OpenXR runtime
over Link, 2026-11-24): both eyes drew DLSS, and the window drew it again
after leaving VR. Not run in VR on Linux.
After a graphics driver reset (the device lost and opened again), DLSS
starts again on the new device, with any earlier refusal forgotten; if
NVIDIA's library refuses it there, FSR draws. Nothing of NVIDIA's is called
on a lost device. If NVIDIA's library is still busy on the lost device
after a second, it is left as it is and the new device opens without DLSS:
FSR draws until the viewer restarts, and Preferences says so. The same
holds when a closing window's render thread stays stuck in the GPU or
the VR runtime with DLSS started on a device that still works: its DLSS
is kept as it is, never ended under it, and the next device opens without
DLSS. If that device was lost, or DLSS had not started, the next device
starts DLSS as usual; if the thread returns later, its DLSS is shut down
then, and the devices after it start DLSS again. When the video memory refuses what DLSS needs, FSR
draws, Preferences says DLSS failed, and the next remake of the targets
(a resize, a setting) tries again.
Film export from the Director keeps the window's DLSS aside while it draws
its frames at their own size, so neither is made again for each frame.
The settings file keeps DLSS as FSR with DLSS on, so a viewer without DLSS
sharing the settings folder (a build without the feature) draws FSR. An
anti-aliasing chosen in such a build turns DLSS off in the settings, so
the DLSS build does not pick it again; only opening Preferences there
leaves it on.
Help › About, the Dev Toolbox and the crash report name what drew the last
frame: NVIDIA DLSS, DLAA or DLSS Ray Reconstruction, or FSR 3.1 while it
stands in for DLSS.
While the window is being resized FSR draws, and DLSS is made once the
size has held for a moment.
With stochastic transparency on, the transparency pass keeps its own
temporal blend before DLSS, which gets no reactive mask for it: moving
blended faces may leave a double trail. Not yet compared side by side.

## What is installed, and where

On Linux the folder is `$AURORA_DLSS_DIR`, else
`$XDG_DATA_HOME/aurora-dlss`, else `~/.local/share/aurora-dlss`:

- `libnvidia-ngx-dlss.so.<version>`, copied from the SDK's
  `lib/Linux_x86_64/rel`, `libnvidia-ngx-dlssd.so.<version>`
  beside it for Ray Reconstruction (optional: the driver ships its own),
  and `libnvidia-ngx-dlssg.so.<version>` for frame generation (optional);
- `libsl_ngx.so`, sl-ngx built against the SDK
  (`make DLSS_SDK=<the SDK>`), and the same as `libsl_ngx.<commit>.so`,
  named after the sl-ngx commit it was built from (its first 12
  characters). It holds NVIDIA's NGX core from the SDK's
  `libnvsdk_ngx.a`: keep it for your own use;
- `NVIDIA-DLSS-LICENSE.txt`, NVIDIA's licence you accepted.

The viewer names its Vulkan instance and device to sl-ngx
(`slSetVulkanInfo`, Streamline's manual hooking).

To update DLSS, run the setup again. To remove it:
`./dlss-setup.sh --remove`. Both refuse an `AURORA_DLSS_DIR` anywhere
inside an SDK checkout.

**After a viewer update.** Each viewer version names the sl-ngx commit it
was made for, and loads `libsl_ngx.<commit>.so` when the folder has it
(else `libsl_ngx.so`). When the folder lacks it, the viewer runs
`dlss-setup.sh --update` in the background at its start. It downloads
NVIDIA's DLSS SDK again (its licence, headers and Linux files, about
60 MB), builds that commit against it, installs sl-ngx under its versioned
name only (keeping the three newest, so viewers of other versions on the
PC keep theirs) and replaces the folder's `libnvidia-ngx-dlss*.so` with
the SDK's. Its output goes to `update.log` in the folder. The new
sl-ngx is used from the next start; until then the viewer runs with the
one it loaded. Preferences › Graphics › Anti-aliasing says it under
**Frame generation**: being built, built (restart to use it), or why not.
The update never shows NVIDIA's licence: when the SDK's licence differs
from the one you accepted, it stops with status 3, and you run
`./dlss-setup.sh` to read it.

One run of the script at a time changes the folder: an install, `--update`
and `--remove` each hold a lock, `.aurora-dlss.dlss-setup.lock` beside the
folder (named after the folder). An install or `--remove` started during
another run waits for it and says so; `--update` does not wait: it exits
with status 4 at once, writing nothing (not even `update.log`), and
Preferences says the update is running. `./dlss-setup.sh --help` lists its
exit statuses.

## Ray Reconstruction

With DLSS on, Preferences › Graphics › Anti-aliasing › **Ray Reconstruction**
(experimental, off by default) has DLSS also denoise the ray-traced
shadows, lights, indirect light and reflections. The scene pass then draws
its guides (normals, roughness, metallic and base colour) beside the image.
Every traced pass keeps its own denoiser, temporal history and à-trous
filter: handed to DLSS unfiltered, the sun's soft shadow (filtered with
the lights') crawled as grain over shadowed skin and washed out a tattoo
blended over it, and the reflections hand DLSS no hit distance or motion of
their own. Its model (Automatic, D, E, F) is chosen under it while it is on;
super resolution's model, changed while it draws, leaves it as it is.
DLSS reads one surface for the depth, the motion and the guides, as
NVIDIA's Ray Reconstruction guide asks: the opaque surface, the impostor
cards (moved by the camera through their depth), and the blended faces
opaque to the eye (heads, bodies, hair that look solid), which give their
own depth, motion and mesh normal, matte, with their colour standing for
their base colour. Fainter blended faces (glass, a soft strand edge) and
the water keep the surface behind them in all three, the guide's choice
for a transparent surface without reflection motion of its own: they are
in the image alone. Deep snow is a known limit: its raised surface is drawn
after the guides are taken, so its edges move with the ground buried under
it. So is an object derezzing: the guides test its texture's alpha
without the fade, so its fading texels keep their guides until they are
gone. Streamline's camera matrices are row major, as NVIDIA's guide
says.

It needs a card and driver with Ray Reconstruction (the choice is greyed
while it is off, otherwise; on, it can always be turned off, and a note
under it says why it cannot draw, or that ray tracing and the shadows must
be on) and NVIDIA's Ray Reconstruction
library. On Linux the driver
ships one (`libnvidia-ngx-builtin-dlssd.so`, in nvidia-utils 620), which
NGX uses when the folder has none; the setup also installs the SDK's copy,
which may be newer. On Windows it comes with Streamline's release. It costs more than super resolution alone. With
ray tracing or the shadows off, or where the card cannot trace, DLSS draws
super resolution and the guides are not drawn; the choice stays saved. If
Streamline refuses Ray Reconstruction, or the video memory refuses its guides or
the render targets only it reads, DLSS falls back to super resolution, and
the log says why; the refusal holds for that card until **Try again**
under it (shown while it is refused, on or off), or another model, asks
Streamline again. A photo or film frame for which Streamline, the video memory or its
evaluate refuses Ray Reconstruction is drawn with super resolution, at once
where it can be (a refused feature or guides), from the next frame or 360°
face otherwise; the frame whose evaluate failed is itself drawn by FSR.
From there every capture, of any size and in any later photo or film,
draws super resolution until **Try again** (shown then too, with a note
under the choice), another model or the next device, while the window
goes on drawing Ray Reconstruction. The tiles of a tiled photo drawn
before the refusal stay Ray Reconstruction's. The pipelines without the
guides are compiled once, at the next capture; the window's frames between
captures draw super resolution with them until no capture has followed for
two seconds, then Ray Reconstruction again, so that a film does not
compile them again at every frame; **Try again** or another model chosen
during a film takes effect then, and the retry they ask during a tiled
photo or a 360° capture at its end, so that its later tiles or faces do
not take up Ray Reconstruction its first drew without (another model
itself applies at once, as any setting changed during a photo does). Pipelines compiled for a settings change while a photo is drawn wait
for its end, the frames between its tiles included: a tiled photo's tiles
and a 360° capture's faces keep the pipelines they began with (a film's
frames each keep theirs). A Vulkan instance or device that lacks Ray
Reconstruction's extensions, or refuses them, is made with super
resolution's alone (the log says which are missing), and DLSS draws super
resolution there.

## Frame generation (Linux)

With DLSS on, Preferences › Graphics › Anti-aliasing › **Frame generation**
(experimental, off by default; Windows: *Frame generation on Windows*,
below) has DLSS make a frame between
each two the viewer draws, shown before the drawn one. The frame is drawn
aside, into a texture in the window's format.

**sl-ngx's presenter** shows the frames when it can. The viewer lets go of
the window's surface and makes a new one, which it hands to sl-ngx
(`slngxPresenterCreate`) without making a swapchain on it. At each frame's
end it hands over the generated frame and the drawn one, in a ring of
three. A thread of sl-ngx, on a queue of its own, copies them into its own
swapchain and presents the generated frame first and the drawn one half a
frame later, under Fifo, with Reflex in low-latency mode where the driver
has `VK_NV_low_latency2`. Turned off, the presenter stops and the window
gets a swapchain of the viewer's again. It needs an sl-ngx with the
presenter (one built from the commit the viewer names: *After a viewer
update*, above) and a second queue in the graphics family.

**The fallback** (the first prototype), when the presenter cannot run: at
the frame's end the generated frame is presented in the window image
acquired at the frame's start, then the drawn one in a second. It has no
Reflex. It runs until the setting is next turned off, except while the
second camera holds the window: the presenter starts once it closes.

**Under the switch**, while it runs, Preferences says what shows the frames
(the presenter, or the fallback and why), whether Reflex is on, and the
mode with the frames shown per second against the frames drawn: the
viewer's own frame rate counts only the drawn ones. The shown frames are
the ones presented: the viewer does not turn on present timing, so they
are not the ones the screen showed.

sl-ngx's presenter runs on Wayland; on X11 frame generation presents the
fallback way, and Preferences says why.

DLSS makes the frame from the drawn one before
the interface; the interface on the generated frame is the drawn frame's,
laid over it wherever the interface drew. While it runs the window presents under Fifo,
whatever the vsync setting (not an `AURORA_PRESENT_MODE` forced for a
test: *For a test*, below), and the frame rate limit stands aside (the
background limit does not): the screen paces the pairs, so the viewer draws
at half the refresh rate and shows a frame at every refresh. It stops
while the viewer is not in front (neither its window nor one of its windows
outside has the focus), so the window presents in the vsync setting's mode
behind another workspace, and starts again with the focus; a capture or an
`AURORA_PROFILE` run keeps it behind. Left running there, the window's first
images once in front again waited up to a second each, for up to 11 s. A frame
generation did not run on (a reset, a gap, a modal's blur, a photo) shows
the drawn frame alone. Turned off, its images are let go and the vsync
setting's mode comes back; NGX's own frame generation stays loaded until
DLSS frees its resources (DLSS turned off, or the viewer closed).

It needs sl-ngx with frame generation (NGX's `FrameGeneration` feature,
and on Linux its presenter, in sl-ngx since `5b0dd82`, the commit
`scripts/dlss-setup.sh` builds; the viewer builds it again at start after
an update), the SDK's
`libnvidia-ngx-dlssg.so.*` beside it, and a card and driver NGX offers it on:
Streamline is asked when DLSS starts, and the choice is greyed while it is
off and not offered (on, it can always be turned off, with a note under it).
The viewer's GPU timers do not see NGX's pass; the log's
`DLSS frame generation:` lines count the runs and the frames presented,
and with the presenter, its own numbers (frames taken and skipped,
swapchains made, its copies' GPU time, low latency, its last error).

## Windows

DLSS on Windows (64-bit, NVIDIA RTX) works the same way, from a build with
the `dlss` feature, with NVIDIA's signed Streamline release. Nothing is
built. Run on Windows by a contributor (RTX 4090, 2026-11-24): DLSS drew
at each mode and model, through a resize, fullscreen and an overlay.

1. Close the viewer (it holds the DLSS files open while it runs), then run,
   in PowerShell, from the viewer's folder:

   ```powershell
   powershell -ExecutionPolicy Bypass -File dlss-setup.ps1
   ```

   It downloads NVIDIA Streamline v2.14.16 from
   <https://github.com/NVIDIA-RTX/Streamline/releases> (about 280 MB),
   checks the zip's SHA-256 and that each DLL is signed by NVIDIA, shows
   you NVIDIA's DLSS licence, and installs once you type `yes`. Already
   downloaded the zip? Add `-Zip <the file>`. To remove DLSS: `-Remove`. A
   relative `AURORA_DLSS_DIR` is read from PowerShell's current folder.
   Both refuse a folder inside an unpacked Streamline release, or one
   that is, or lies under, a junction or a symbolic link (the default
   folder too, when the profile is moved through one: then set
   `AURORA_DLSS_DIR` to another folder).
2. Restart the viewer and choose **NVIDIA DLSS** as above.

The folder is `%AURORA_DLSS_DIR%`, else `%LOCALAPPDATA%\aurora-dlss`. These
files come from the release's `bin\x64`, unchanged:

- `sl.interposer.dll`, `sl.common.dll`, `sl.dlss.dll`: NVIDIA Streamline;
- `nvngx_dlss.dll`: the DLSS model;
- `sl.dlss_d.dll` and `nvngx_dlssd.dll`: the same, for Ray Reconstruction;
- `sl.dlss_g.dll` and `nvngx_dlssg.dll`: frame generation, with
  `sl.reflex.dll`, `sl.pcl.dll` and `NvLowLatencyVk.dll`, the Reflex it
  needs (*Frame generation on Windows*, below; files installed by an earlier
  run of the script lack them: run it again);
- `NVIDIA-DLSS-LICENSE.txt`, NVIDIA's licence you accepted, and
  `STREAMLINE-LICENSE.txt`, Streamline's own (MIT).

The viewer makes its Vulkan instance through `sl.interposer.dll`, so
Streamline sees the instance and device it makes. Streamline holds one
device at a time and keeps it until it is shut down. So on Windows, after a
graphics driver reset, DLSS stays off until the viewer restarts: FSR draws,
and Preferences says so. The VR headset opens its own Vulkan instance and
device through `sl.interposer.dll` too: Streamline is shut down on the
window's device first and starts again on the headset's. Streamline normally also watches the frames
presented. The viewer calls DLSS without that, which Streamline allows,
but whether its long-running cleanup then runs has not been checked.

### Frame generation on Windows (experimental)

Preferences › Graphics › Anti-aliasing › **Frame generation** works on
Windows too, through NVIDIA's own DLSS frame generation in Streamline rather
than the Linux prototype's two presents: Streamline hooks the window's
present (the viewer's Vulkan calls already go through `sl.interposer.dll`)
and presents the generated frames itself, paced on its own queue, with
Reflex keeping input latency down. Not yet run on Windows.

- **A restart to turn it on.** Streamline loads frame generation when it
  starts, with the viewer's GPU, or not at all; loaded, every swapchain goes
  through its proxy, which costs a copy even with frame generation off. So
  the setting is read as the viewer starts, and Preferences says when a
  change waits for a restart. Off at that start, nothing of it is loaded,
  and the switch can be turned on whether or not the card has it: the
  restart tells. Turned off while it runs, it stops at once; the proxy goes
  at the next restart.
- **What it is given.** For each frame DLSS draws: the depth and motion
  DLSS reads, copied by the viewer before DLSS runs, and the window's image
  before the interface (a copy of it, in the window's format), all three
  tagged valid until the present, so Streamline copies none of them. The
  interface itself is not given, so in a generated frame it is moved with
  the scene behind it where they differ.
- **Off for a frame** DLSS did not draw, under a modal's blur, and before
  the window's size or vsync changes (NVIDIA's guide: not while the
  swapchain is made again): the swapchain is made again once a present has
  asked it off, or after three frames, or at once when the window gives no
  image, asked off first. Off too in a window under the smallest size it
  takes, and after three presents in a row whose status is not `ok`, until
  the window takes another size. Off as DLSS goes (another anti-aliasing,
  another model), its memory let go.
- **Vsync is not applied** while it runs: NVIDIA supports vsync with frame
  generation on Direct3D 12 only. The frame rate limit stays in effect, and
  limits the frames drawn.
- **Needs** an RTX 40 or 50 card, Windows 10 20H1 or later, and
  hardware-accelerated GPU scheduling on (Windows' Graphics settings).
  RTX 40 makes one frame per frame drawn (2x); `AURORA_DLSS_FG_FRAMES=<n>`
  asks for `n` per frame drawn for a test, at most what the card says,
  read before the first ask (the log's `up to <n>x here`).
- **The log.** `DLSS: frame generation supported here` (or why not) at the
  start; `DLSS frame generation: on` when it is first asked for; then at the
  first present with it, each 600th and at each change, its status (`ok`,
  or what Streamline refused) and the frames presented per frame drawn.
- Other windows of the viewer that present (a window sent outside it) go
  through Streamline's proxy too; only the main window is given frame
  generation.

## Which DLSS version

The version is that of NVIDIA's files, not of the viewer:

- **Linux:** the setup takes the newest DLSS SDK on
  <https://github.com/NVIDIA/DLSS> when it runs (310.9.16 on 2026-11-25).
  The installed file's name gives it: `libnvidia-ngx-dlss.so.<version>`.
  Run the setup again for a newer one.
- **Windows:** the `nvngx_dlss.dll` of the Streamline release the setup
  pins (v2.14.16). Its file properties give the version.

"DLSS 4.5" is NVIDIA's name for its second-generation transformer models,
L and M, added in DLSS SDK 310.5.0; K is the DLSS 4 transformer model
([NVIDIA](https://www.nvidia.com/en-us/geforce/news/dlss-4-5-super-resolution-available-now/)).
On Automatic, NVIDIA's defaults pick K for DLAA, Quality and Balanced, M
for Performance and L for Ultra Performance (`nvsdk_ngx_defs.h` in the
SDK); NVIDIA may change them in a driver update. Choosing L or M uses
them in every mode. RTX 20 and 30 cards lack FP8, so L and M cost them
more than K does.

## For a test

- `AURORA_DLSS=1` turns DLSS on for one launch, whatever the DLSS
  setting; it draws in FSR's place, so the anti-aliasing must be FSR too
  (`AURORA_AA=7`, for example `AURORA_DEMO=1 AURORA_AA=7 AURORA_DLSS=1`).
  `AURORA_DLSS=0` keeps DLSS and NVIDIA's files out of that launch entirely.
  Starting a viewer from a build with the feature, and without the files,
  is the same as a build without it: the log says so, and FSR draws.
- `AURORA_DLSS_DIR=<folder>` looks for the files in another folder.
- `AURORA_DLSS_PRESET=<n>` uses NVIDIA's preset number `n` whatever the
  setting (11 K, 12 L, 13 M; 0 NVIDIA's own choice).
- `AURORA_DLSS_RR=1` or `0` turns Ray Reconstruction on or off for one
  launch, whatever the setting; `AURORA_DLSS_RR_PRESET=<n>` uses NVIDIA's
  Ray Reconstruction preset `n` (4 D, 5 E, 6 F; 0 NVIDIA's own).
  `AURORA_DLSS_RR_FAIL=1` has the viewer refuse Ray Reconstruction as if
  Streamline had, to check the fall back to super resolution. Its other debug
  variables (`AURORA_DLSS_RR_SHOW`, `AURORA_DLSS_RR_TRANSPOSED`,
  `AURORA_DLSS_RR_BEHIND`) and the traced passes' denoisers
  (`AURORA_RT_RAW`, `AURORA_RT_GI_RAW`) are in docs/TESTING.md.
- `AURORA_DLSS_FG=1` or `0` turns frame generation (*Frame generation*,
  above) on or off for one launch, whatever the setting. With it on, an
  `AURORA_PRESENT_MODE` other than `fifo` (and one the window offers)
  keeps sl-ngx's presenter from starting: the viewer presents both frames
  itself, in that mode, and Preferences says why.
  `AURORA_DLSS_FG_DUMP=<folder>` writes three frames' images as PPM
  (docs/TESTING.md). On Windows `AURORA_DLSS_FG_FRAMES=<n>` asks for `n`
  generated frames per frame drawn (1 by default).
- The log's `DLSS:` lines say what was found and why DLSS did not start.
