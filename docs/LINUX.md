# Installing Aurora Viewer on Linux

Every release of Aurora Viewer comes with a Linux build: an archive to
extract and run, with nothing to install as root. It is tested on Arch Linux
and CachyOS; other recent distributions work the same way.

## Requirements

- A 64-bit PC (x86-64) and a graphics card with a **Vulkan** driver.
- A recent distribution: the viewer is built on Ubuntu 24.04 and needs glibc
  2.39 or later (Arch, CachyOS, Manjaro, Fedora 40 or later, Ubuntu 24.04 or
  later, Debian 13…).
- X11 or Wayland, and PipeWire, PulseAudio or ALSA for the sound. When a
  PipeWire (with `pipewire-pulse`) or PulseAudio server runs, the viewer
  plays and records through it, so its own sinks and sources appear in
  Preferences; otherwise it uses ALSA directly. No extra package is needed.

## 1. Install the system packages

The viewer itself needs Vulkan, the sound and the windowing libraries. Web
pages, YouTube and video on media faces also need the libraries of the
built-in browser and **GStreamer** for video, as in the Legacy Viewer.

**Arch Linux, CachyOS, Manjaro**

```bash
sudo pacman -S --needed vulkan-icd-loader alsa-lib libxkbcommon wayland libx11 libxcursor libxrandr libxi \
  gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav \
  nss nspr at-spi2-core libcups libxcomposite libxdamage mesa pango cairo
```

**Debian, Ubuntu**

```bash
sudo apt install libvulkan1 libasound2t64 libxkbcommon0 libwayland-client0 libx11-6 libxcursor1 libxrandr2 libxi6 \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav \
  libnss3 libatk-bridge2.0-0t64 libcups2t64 libxcomposite1 libxdamage1 libgbm1 libpango-1.0-0 libcairo2
```

Plus the Vulkan driver of your graphics card if it is not there yet:
`vulkan-radeon` (AMD), `vulkan-intel` (Intel) or the NVIDIA driver on Arch;
`mesa-vulkan-drivers` or the NVIDIA driver on Debian and Ubuntu.

## 2. Download and extract

From the [latest release](https://github.com/Aurora-Viewer/Aurora-Viewer/releases/latest),
download `Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz` and its `.sha256` file into
the same folder, then:

```bash
sha256sum -c Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz.sha256
tar -xzf Aurora-Viewer-vX.Y.Z-linux-x64.tar.gz
```

The archive holds one folder, `Aurora-Viewer-vX.Y.Z-linux-x64`, with the
viewer (`aurora-viewer`), its desktop entry and icon, its licences and the
README. Move it where you like, for example `~/.local/opt/aurora-viewer`.

## 3. Start the viewer

```bash
cd Aurora-Viewer-vX.Y.Z-linux-x64
./aurora-viewer
```

To start it from your desktop's menu, with its icon in the panel and the
dock, install the desktop entry and the icon of the archive, from its
folder. The entry gets the viewer's real path, quoted and escaped as the
desktop entry specification asks, so a folder name with spaces or symbols
works:

```bash
install -Dm644 aurora-viewer.svg ~/.local/share/icons/hicolor/scalable/apps/aurora-viewer.svg
mkdir -p ~/.local/share/applications
exe=$(printf '%s' "$PWD/aurora-viewer" | sed -e 's/[\\"`$]/\\&/g' -e 's/\\/\\\\/g' -e 's/%/%%/g')
EXEC="\"$exe\"" awk '/^Exec=/ { print "Exec=" ENVIRON["EXEC"]; next } 1' aurora-viewer.desktop > ~/.local/share/applications/aurora-viewer.desktop
```

Run the last two lines again if you move the folder. Avoid a `%` in the
folder's path: GNOME and other GLib desktops then refuse the entry. The
window identifies itself as `aurora-viewer` (Wayland app_id and X11
`WM_CLASS`), the name of the entry and its `StartupWMClass`: that is how
the desktop puts the window under the entry's icon. Compositor rules can
match it too (Hyprland: `class:^(aurora-viewer)$`; KDE: window class
`aurora-viewer`).

## Web pages and video

The first web page or video downloads the media plugins (about 200 MB) by
itself, once; Preferences › Sound and voice › Media shows the progress. They
go to `~/.local/share/auroraviewer/media-plugins`.

- **Video does not play:** check that the GStreamer packages of step 1 are
  installed (`gst-libav` / `gstreamer1.0-libav` for H.264 and AAC).
- **A web page stays blank:** this command lists the browser libraries that
  are missing; install their packages.

  ```bash
  ldd ~/.local/share/auroraviewer/media-plugins/*/llplugin/libcef.so | grep "not found"
  ```

- The built-in browser runs without Chromium's sandbox, as in the Legacy
  Viewer, unless `chrome-sandbox` (in the same `llplugin` folder) belongs to
  root and has the setuid bit.

## Updates

The viewer tells you when a new version is out, but it cannot replace
itself on Linux yet: its "Open the release page" button leads to the new
archive. Extract it as in step 2, in place of the old folder or next to it.
Your settings, cache and logs are kept: they are not in the viewer's folder.

| What | Where |
|---|---|
| Settings | `~/.config/auroraviewer` |
| Cache | `~/.cache/auroraviewer` |
| Logs, media plugins | `~/.local/share/auroraviewer` |

## Second Life links

Preferences › Language and updates › "Open SLURL and Hypergrid links in
Aurora" makes Aurora the handler of `secondlife://`, `hop://` and
`x-grid-location-info://` links for the current user. It writes a hidden
desktop entry, `~/.local/share/applications/aurora-viewer-slurl.desktop`,
and makes it the default for the three schemes in `~/.config/mimeapps.list`;
the rest of that file is kept. Turning it off removes both. A link opened
while Aurora runs goes to that viewer over the session bus instead of
starting a second one. Wayland does not let it bring its window to the
front, so the window stays where it is (X11 is not tested yet).

A desktop-specific list (`~/.config/<desktop>-mimeapps.list`, such as
`kde-mimeapps.list`) comes first: if it names another program for these
links, Preferences names that program instead.

`xdg-open` (from xdg-utils) cannot start a program whose path holds a
space or a symbol such as `'`, `&` or `(`: it then opens the link in
another viewer. Keep the viewer's folder path plain, or open links through
your desktop (GNOME, KDE and other GLib desktops read such paths).

## Not on Linux yet

- Automatic installation of updates (above).
- The crash report window.

## Building from source

```bash
sudo pacman -S --needed git base-devel rustup cmake clang pkgconf alsa-lib libxkbcommon wayland libx11 libxcursor libxrandr libxi vulkan-icd-loader
rustup default stable
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
cd aurora-viewer
cargo build --release -p aurora-viewer
./target/release/aurora-viewer
```

On Debian and Ubuntu, the build packages are `build-essential cmake clang
pkg-config libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev`, with
Rust from [rustup.rs](https://rustup.rs).

## Reporting a problem

See [Reporting bugs](BUG_REPORTING.md), and say that you are on Linux, with
your distribution and graphics card.

## Experimental VR scene viewing (Linux/OpenXR)

This is **viewing only**, anchored at the desktop camera's position when VR starts.
Place the camera where you want to look from **before** entering VR. Its horizontal
heading is preserved and its pitch/roll are leveled to the horizon. Head rotation
and small physical movements provide stereo head tracking around that anchor;
they do not move the avatar or provide locomotion around the world.

### Requirements and headset compatibility

- Linux desktop viewer with a Vulkan 1.3 driver/GPU that meets Aurora's existing
  renderer feature and bindless resource requirements.
- A native OpenXR loader (`libopenxr_loader.so.1`; install your distribution's
  OpenXR loader package) and a configured, running Linux OpenXR runtime.
- Runtime support for `XR_KHR_vulkan_enable2`, a head-mounted display with
  `PRIMARY_STEREO` (two views), `LOCAL` and `VIEW` reference spaces, opaque
  composition, and the viewer's selected RGBA8/BGRA8 output format. Eye swapchains
  must support color attachment, sampling and transfer-source usage.
- The runtime and headset must be available **before launching Aurora**. With
  WiVRn, start its server/dashboard and connect the headset first; verify its
  active OpenXR runtime registration. Then launch Aurora normally. No special
  demo variable is needed for normal use.

**Validated:** Meta Quest 3 through WiVRn 26.9 on Linux with an NVIDIA RTX 3090.
Offline demo and human-operated live-grid viewing have been tested, including
repeated entry/exit, horizon alignment, head tracking and the desktop mirror.

**Not a promise that every OpenXR headset works.** The implementation uses the
OpenXR API rather than a Quest-specific API, so another Linux runtime/headset
meeting these requirements is a candidate for testing. Other headsets, GPUs,
Monado/SteamVR runtime configurations and multi-GPU systems remain unvalidated.
OpenXR branding alone does not establish compatible graphics bindings, formats
or runtime behavior. macOS, standalone Android builds, OpenVR-only
applications/runtimes and passthrough/AR are outside this iteration.
See the [OpenXR Vulkan binding specification](https://registry.khronos.org/OpenXR/specs/1.1/man/html/xrGetVulkanGraphicsRequirements2KHR.html)
and [WiVRn project](https://github.com/WiVRn/WiVRn) for runtime information.

Windows uses the same experimental Vulkan/OpenXR code path, but Windows headset
validation is pending. See [Windows VR setup and validation](VR_WINDOWS.md).

### Entering and leaving VR

Click the headset button in the bottom toolbar. Press **Esc**, with the Aurora
window focused, to return to the original desktop camera and interface. Re-enter
with the toolbar button without restarting the viewer while the runtime remains
available. A desktop-only instruction reminds you how to exit.

### Deliberate limits of this first iteration

- **No VR interface or usable interaction controls:** no menus, chat, inventory,
  building/editing, object interaction, movement controls, controllers, hands or
  VR shortcuts. Exit VR to interact with the viewer or change camera placement.
  Esc is the supported keyboard action while VR is engaged.
- **The viewpoint stays anchored where the desktop camera was placed.** Head
  tracking permits looking around and physical leaning; there is no VR travel,
  camera repositioning, avatar movement or controller teleportation.
- Headset output has no viewer interface, nameplates, chat overlays, worn HUDs,
  selection outlines or editing overlays. Desktop output is a left-eye mirror
  with the exit instruction, not a second interactive desktop viewport.
- Starting Aurora before the runtime/headset is ready can leave VR unavailable
  for that process. Connecting later and recovering from a runtime restart or
  headset disconnect are unfinished; a viewer restart may be needed. Repeatable
  normal enter/exit does not imply disconnect/reconnection support.
- Each eye is capped at 1600 pixels in each dimension. Two eye backends maintain
  independent depth, visibility and temporal history; this uses additional GPU
  time and memory. Multiview rendering and comprehensive stereo-effects testing
  are follow-up work. No minimum frame rate or measured overhead is claimed.
- Esc is not a global shortcut: refocus Aurora if another application owns the
  keyboard. Do not rely on a headset controller to leave VR.

### Dependencies and footprint

The only new direct Rust dependency is Linux/Windows-only `openxr 0.22.0` (MIT/Apache-2.0).
It adds `openxr-sys 0.14.0` (MIT/Apache-2.0) and `libloading 0.9.0` (ISC); the
existing `libloading 0.8.9` remains for other dependencies. These licenses are
compatible with this repository. The loader is dynamically loaded: the system
OpenXR loader and runtime are installed separately, not bundled in Aurora.

No VR framework/engine, WiVRn, SteamVR, OpenVR compatibility layer, OBS, video
encoder or headset client is added to the application. Existing wgpu/Vulkan and
egui infrastructure is reused. The `xr_spike` example is development-only and is
not part of the viewer executable. Dependency count does not measure rendering
cost: stereo rendering, per-eye resources and capture/encoding have real cost;
a controlled binary-size/GPU-memory/frame-time comparison is still pending.

### Recording VR

Record the desktop mirror with OBS Studio using **Window Capture (PipeWire)**
on Wayland (or the corresponding window source on X11), selecting the Aurora
window. Use a hardware encoder such as NVIDIA NVENC H.264 when available.
The mirror reuses an eye image and adds a fullscreen GPU pass, rather than
rendering another scene or reading the image back to the CPU. It is capped at
30 FPS and letterboxed to preserve the eye's aspect ratio. Capture and video
encoding have additional overhead; start with 30 FPS recording.

The desktop exit instruction appears in recordings. The mirror records
application-rendered frames before headset streaming/compositor processing, so
streaming-only defects may not appear. OBS is optional and installed separately.
