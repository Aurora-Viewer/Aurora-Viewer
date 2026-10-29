# Installing Aurora Viewer on Linux

Every release of Aurora Viewer comes with a Linux build: an archive to
extract and run, with nothing to install as root. It is tested on Arch Linux
and CachyOS; other recent distributions work the same way.

## Requirements

- A 64-bit PC (x86-64) and a graphics card with a **Vulkan** driver.
- A recent distribution: the viewer is built on Ubuntu 24.04 and needs glibc
  2.39 or later (Arch, CachyOS, Manjaro, Fedora 40 or later, Ubuntu 24.04 or
  later, Debian 13…).
- X11 or Wayland, and ALSA or PipeWire / PulseAudio for the sound.

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
README. Move it where
you like, for example `~/.local/opt/aurora-viewer`.

## 3. Start the viewer

```bash
cd Aurora-Viewer-vX.Y.Z-linux-x64
./aurora-viewer
```

To start it from your desktop's menu, with its icon in the panel and the
dock, install the desktop entry and the icon of the archive, from its
folder (the entry gets the viewer's real path):

```bash
install -Dm644 aurora-viewer.svg ~/.local/share/icons/hicolor/scalable/apps/aurora-viewer.svg
mkdir -p ~/.local/share/applications
sed "s|^Exec=.*|Exec=$PWD/aurora-viewer|" aurora-viewer.desktop > ~/.local/share/applications/aurora-viewer.desktop
```

Run the `sed` line again if you move the folder. The window identifies
itself as `aurora-viewer` (Wayland app_id and X11 `WM_CLASS`), the name of
the entry and its `StartupWMClass`: that is how the desktop puts the
window under the entry's icon. Compositor rules can match it too (Hyprland:
`class:^(aurora-viewer)$`; KDE: window class `aurora-viewer`).

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

## Not on Linux yet

- Automatic installation of updates (above).
- Desktop notifications and the crash report window.
- Opening `secondlife://` links from a web browser.

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
