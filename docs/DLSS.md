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
frame: NVIDIA DLSS or DLAA, or FSR 3.1 while it stands in for DLSS.
While the window is being resized FSR draws, and DLSS is made once the
size has held for a moment.
With stochastic transparency on, the transparency pass keeps its own
temporal blend before DLSS, which gets no reactive mask for it: moving
blended faces may leave a double trail. Not yet compared side by side.

## What is installed, and where

On Linux the folder is `$AURORA_DLSS_DIR`, else
`$XDG_DATA_HOME/aurora-dlss`, else `~/.local/share/aurora-dlss`:

- `libnvidia-ngx-dlss.so.<version>`, copied from the SDK's
  `lib/Linux_x86_64/rel`;
- `libsl_ngx.so`, sl-ngx built against the SDK
  (`make DLSS_SDK=<the SDK>`). It holds NVIDIA's NGX core from the SDK's
  `libnvsdk_ngx.a`: keep it for your own use;
- `NVIDIA-DLSS-LICENSE.txt`, NVIDIA's licence you accepted.

The viewer names its Vulkan instance and device to sl-ngx
(`slSetVulkanInfo`, Streamline's manual hooking).

To update DLSS, run the setup again. To remove it:
`./dlss-setup.sh --remove`. Both refuse an `AURORA_DLSS_DIR` anywhere
inside an SDK checkout.

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
- The log's `DLSS:` lines say what was found and why DLSS did not start.
