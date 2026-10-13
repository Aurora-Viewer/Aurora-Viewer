<div align="center">

<img src="assets/branding/logo-loup-couleur.svg" alt="Aurora Viewer" width="180">

# Aurora Viewer

**A modern Second Life viewer written in Rust.**

[![CI](https://github.com/Aurora-Viewer/Aurora-Viewer/actions/workflows/ci.yml/badge.svg)](https://github.com/Aurora-Viewer/Aurora-Viewer/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Aurora-Viewer/Aurora-Viewer?include_prereleases&color=8B5CF6)](https://github.com/Aurora-Viewer/Aurora-Viewer/releases)
[![License: GPLv3](https://img.shields.io/badge/license-GPLv3-4F46E5)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-5EEAD4?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Windows-070B1F)](#build)

[Aurora vs Firestorm](#how-aurora-differs-from-firestorm) · [Roadmap](#roadmap) · [Build](#build) · [Contributing](#contributing)

</div>

![Aurora Viewer in Second Life](docs/screenshots/scene.jpg)

Aurora Viewer is a Second Life client built from scratch in Rust, with a
modern GPU renderer (wgpu on Vulkan) and a clean, flat interface. It follows
the behaviour of [Firestorm](https://www.firestormviewer.org) — same protocol,
same rules, same defaults — while rethinking the user interface. AMD FSR
upscaling and anti-aliasing are planned (see the [roadmap](#roadmap)).

> **Status:** early development (0.x). Usable for exploring, chatting and
> testing; many features are still in progress — see [TASKS.md](TASKS.md).

## How Aurora differs from Firestorm

**What it is**

- A Second Life viewer written from scratch in Rust; it is not a fork of
  the Linden or Firestorm codebase.
- It speaks the same protocol as Firestorm (same messages, capabilities,
  request rates and defaults), so the servers see a well-behaved, familiar
  client.
- Open source (GPLv3), with credit to the Linden / Firestorm code
  it ports logic from (see [NOTICE.md](NOTICE.md)).

**Rendering engine**

- Vulkan (through wgpu) instead of OpenGL.
- The GPU does the heavy lifting: it decides what is visible (culling in
  compute shaders) and builds its own draw lists.
- Bindless textures and multi-draw-indirect let thousands of objects be
  drawn in a handful of calls.
- A dedicated render thread draws frame N while the main thread already
  prepares frame N+1.
- Texture and mesh decoding runs on background threads and writes straight
  into GPU staging memory, so arriving in a busy region doesn't freeze the
  picture.
- Textures are kept BC7-compressed in video memory when that leaves them
  visually intact (each one is checked), about half the texture memory
  overall, and cached that way on disk: back in a place already visited,
  they go to the GPU without being decoded again.
- Same look as Firestorm: PBR and legacy materials, EEP sky, shadows,
  reflection probes, mirrors, water, glow and particles, checked side by
  side on the main grid.

**Performance**

<table>
  <tr>
    <td width="50%" align="center" valign="top">
      <img src="docs/screenshots/compare-firestorm.png" alt="Firestorm statistics window in a busy store: 68 fps"><br>
      <b>Firestorm</b> · 68 fps
    </td>
    <td width="50%" align="center" valign="top">
      <img src="docs/screenshots/compare-aurora.png" alt="Aurora Viewer performance window in the same store: 386 fps"><br>
      <b>Aurora Viewer</b> · 386 fps
    </td>
  </tr>
</table>

Measured on 10 October 2026 with the frame cap off, on one PC (RTX 4090), in
one busy store of about 16,000 objects. This is an informal test, not a
benchmark:

- 138 fps where Firestorm gave 68 fps at the same spot;
- about 343 fps at that spot (median) once the render thread landed, 386 fps
  in the capture above.

The texture memory budget is set automatically from the detected VRAM.

**Interface**

- A flat, modern interface of our own: a single top bar, a notification
  center, searchable conversations and a redesigned performance window.
- Teleport and loading screens show the blurred last view and real
  step-by-step progress.
- Teleports within the same region are instant, with no loading screen.
- Movement keys adapt to the keyboard layout on first launch (WASD or ZQSD).
- The interface is available in English and French: system language
  detection, a choice saved in Preferences and applied at once. Fluent
  catalogues make it possible to add a language; see
  [the translation guide](docs/TRANSLATING.md).
- Double-click a window's title to collapse it into a movable bar, then
  restore it with its size and content.
- On closing: a dialog over a blurred background with Quit, Log out (back to
  the login screen) and Cancel. Esc cancels; the viewer waits for the logout
  before closing.

**Things Firestorm doesn't have**

- Link safety check: web links in chat and profiles are marked trusted,
  unknown or dangerous, judged locally with nothing sent anywhere.
- Parcel permission icons show everything, green for allowed and red for
  forbidden; Firestorm shows only the restrictions.
- HUDs can be moved directly with Alt + drag (the key is configurable),
  without opening the build tools. Hold the same key and scroll over a HUD
  to resize it locally (Shift gives finer steps), including no-modify HUDs.
  Sizes from 10% to 800% are saved per account by inventory name and creator,
  then restored on new copies. Right-click a HUD to reset its size to 100%
  or forget its saved size (the current copy keeps its size).
  While editing a HUD, scrolling without the HUD key zooms the entire HUD
  view, revealing off-screen HUDs while keeping their saved local sizes.
- Local chat bubbles show the speaker's profile picture.
- The remembered password is kept in Windows Credential Manager, as a hash
  only.
- An offline demo mode simulates a server locally, so the viewer can be
  tested with no account and no grid connection.

## What works today

- **Rendering** — wgpu / Vulkan, bindless textures, multi-draw-indirect,
  PBR and legacy materials, shadows, reflection probes, water, glow,
  particles, GPU occlusion (experimental).
- **Avatars** — skeleton and Bento animations blended like Firestorm,
  server bakes, shape, rigged mesh, complexity limits, impostors, look-at.
- **World** — prims, sculpts, mesh with LOD, terrain, EEP environment,
  world sounds, parcel music, media on prims.
- **Social** — local chat, IMs and groups, people list, blocking,
  notifications, voice (WebRTC); conversation history kept on disk per
  account and grid in Firestorm's text format, shown greyed when a
  conversation opens, with search and a History window; instant messages
  received while offline.
- **Interface** — flat Aurora theme, Tabler icons, mini-map and world map,
  inventory, preferences, debug overlays.
- **Windows updates** — release checks, cancellable verified downloads,
  installation and restart with rollback. The prompt explains whether cache
  and settings will be preserved, migrated or reset.

![Login screen](docs/screenshots/login.jpg)

## Reporting a bug during the alpha

The bug icon in the top bar, the Help menu, the login screen and the "Send a
crash report" button after a crash open the same form. After a crash, the
data comes from the session that stopped. Describe the problem and check the
information in the Data tab; untick what you want to keep private. The
interface follows the chosen language, even with the form open. The
screenshot and the filtered diagnostics are optional. The file meant for
GitHub uses English headings; your description stays in the language you
typed it in. A new report opens on Description. Closing then reopening the
form resumes the draft at the same place until it is sent or the viewer is
closed. In browser mode, click "I have published the report" after publishing
on GitHub to start another report. After a screenshot taken in Aurora,
ticking or unticking "Include the interface" retakes the picture
automatically; imported files stay unchanged.

Two modes, both with a GitHub account: "Send from Aurora", shown first and
selected by default, or opening a pre-filled issue in the browser. Sending
from Aurora asks you to authorise its GitHub application; the authorisation
stays in memory only, until the viewer is closed or logged out; nothing is
published automatically after the authorisation. Without an account, save a
local copy.

Sending from Aurora publishes the chosen image on auroraviewer.com and shows
it in the issue. A separate option of the Data tab, unticked by default,
attaches the ZIP of the logs for the developers; its link requires a GitHub
login authorised in the private repository. The files stay in the private
Cellar. An interrupted upload can be finished on the same issue. In the
browser, add the image by hand from the `public` folder. If the text is too
long for a link, Aurora copies it in full so that you can paste it into
GitHub. Never publish the `private-do-not-upload` folder. The bar button can
be hidden in Preferences › Interface.

Offline test: `AURORA_DEMO=1` and `AURORA_DEMO_BUG_REPORT=1` open a sample
report; sending is simulated and the browser stays closed. See
[TESTING.md](docs/TESTING.md) and the [GitHub configuration](docs/BUG_REPORTING.md).

## Roadmap

**Not there yet**

- Snapshots, gestures, RLVa, the shape and clothing editors, group
  profiles, and the full sky / water editors.
- Voice is WebRTC only (no Vivox).
- Linux and macOS builds: the code is portable but not validated.

**Planned**

- AMD FSR (FidelityFX Super Resolution) upscaling and native anti-aliasing,
  on every GPU vendor.
- Frame Generation (under investigation).
- Compression for the textures that fast BC7 encoding would harm (they
  stay uncompressed and are decoded at each visit).
- PBR terrain, projector lights and a full pass on water rendering.
- Maybe a VR mode with controller support.

The detailed list of what is done, in progress and to do is in
[TASKS.md](TASKS.md).

## Build

Requirements: Windows 10/11, [Rust](https://rustup.rs) (stable, selected
automatically by `rust-toolchain.toml`), a GPU with Vulkan support.

**Development environment:** create a folder for the project, clone this
repository into it, then double-click **`aurora-tools.cmd`**:

```bat
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
```

The tools check the environment and offer to **repair** it: they install
whatever is missing (GitHub CLI, the Visual Studio C++ tools, Rust), clone
the Firestorm sources next to the repository, and download the emoji font,
the Rust toolchain and the crates. Then, driven by the keyboard: release /
dev / debug viewers, demo, agents' tasks, disk, logs, live pull requests with
Windows notifications, releases (see
[HUMANS.md](HUMANS.md)).

Without Git yet, this command (in a command prompt, in the project folder)
installs everything, Git included:

```bat
powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"
```

Or by hand, to build the viewer only:

```powershell
git clone https://github.com/Aurora-Viewer/Aurora-Viewer aurora-viewer
cd aurora-viewer
./scripts/fetch-assets.ps1          # emoji font (too large for git)
cargo build --release -p aurora-viewer
./target/release/aurora-viewer.exe
```

Run the same checks as the CI with `./scripts/check.ps1` (rustfmt, clippy
with warnings as errors, tests). Day-to-day development uses the default
profile (`cargo run -p aurora-viewer`): it behaves like the release build
without its slow link-time optimization. `./scripts/build-release.ps1`
builds GitHub's latest `main` with `--release` into `..\RELEASE\`.

The **Git tag is the reference for the viewer's version**: `v0.2.0` gives
`0.2.0` everywhere (title, interface, logs and network clients). Between two
tags, `0.2.0+7.gabcdef123` gives the number of commits and the short
identifier. The version of the crates in `Cargo.toml` is not the viewer's
number. A source archive without Git can be built with
`AURORA_VERSION_TAG=v0.2.0`; otherwise it identifies itself as
`0.0.0+0.gunknown`. The Release workflow checks the version announced by the
exe before publishing; the local build also writes it to `version.txt`.

### Command line

| Argument | Effect |
|---|---|
| `--title <text>` | Names the window "Aurora Viewer <version> - <text> (Dev)" and its log file |
| `--fps-limit <fps>` | Caps this instance at 1–500 fps without changing saved preferences |
| `--language <code>` | Forces the language of this instance (`en`, `fr`, or another language added to the catalogues), without changing the saved choice |
| `-h`, `--help` | Shows the help |
| `-V`, `--version` | Prints the built version then exits, with no window, log or network |

- `--title`: the build profile ends the title (Dev, Release, Debug); the log
  file is `aurora-<text>.log` (`aurora-demo-<text>.log` in demo mode).
- `--fps-limit`: applies an extra limit to this process, including captures
  and online sessions; a lower user preference remains effective.
- `--language`: also accepts `--language=fr`. An unavailable language falls
  back to English; the argument is meant for demos and screenshots.

The viewer first follows the language saved in **Preferences › Interface ›
Language**, then the system's if "System language" is selected. English is
the fallback. The change applies at once; the "Language visible to scripts"
setting is on by default, like Firestorm. The translations are in
`crates/aurora-i18n/locales/`: [translation guide](docs/TRANSLATING.md).

The frame limiter is enabled at **120 fps by default** and can be disabled
or adjusted in Preferences › Graphics › Smoothness. Saved preferences are
preserved.

### Animation Overrider (AO)

The **AO** button of the bottom bar opens the built-in Animation Overrider.
Sets are kept under `#Firestorm/#AO`; import a ZHAO notecard from the same
folder as its animations. The AO is off at first, as in Firestorm. `cao on` /
`cao off` turn it on or off; `cao sit` toggles the sit, `cao sit off` replaces
the sit and `cao sit on` keeps the standard sit, like Firestorm. The demo
scenario is described in
[the test options](docs/TESTING.md#avatars-and-animations).

### Notecards

Notecards show images, landmarks and other embedded items with their icon and
name in the text. Drag an item from the inventory to insert it; a left click
opens it directly, and the right click offers Open or Copy to inventory. Save
(Ctrl+S) keeps their permissions. The demo scenarios are described in
[the testing guide](docs/TESTING.md#interface).

### Test switches

The compact build inspector also accepts `AURORA_DEMO_BUILD_TAB=transform|shape`;
`object` remains an alias for the transformation section. Window redesign
scenarios use the existing inventory, appearance, land, profile, environment
and preferences switches listed in [docs/TESTING.md](docs/TESTING.md).

Local and editing HUD zoom scenarios (`AURORA_DEMO_HUDS=scale|scale-copy|scale-touch|scale-reset|scale-forget|scale-ctrl|scale-key|scale-secondary|edit-scroll`)
are described in [the testing guide](docs/TESTING.md#huds).

The offline **demo mode** simulates a server locally (a plaza, avatars,
objects, chat, notifications…): no connection, no account. It keeps its own
settings and cache (`…\config\demo`, `…\cache\demo`).

```powershell
$env:AURORA_DEMO = "1"
cargo run -p aurora-viewer
```

| Variable | Effect |
|---|---|
| `AURORA_DEMO=1` | Offline demo mode |
| `AURORA_DEMO_<NAME>=…` | A demo scenario: a window, a stress scene, a camera script… |
| `AURORA_DEMO_PIE=1` / `0` | Forces the pie menu / the list menu in the demo, without changing the preference; combine with `AURORA_DEMO_RCLICK` |
| `AURORA_CAPTURE=<file.png>` | Saves a capture of a frame |
| `AURORA_PROFILE=1` | Writes one profiling line per second to the log |

The script editor demos (`AURORA_DEMO_SCRIPTS`): `close-brace` /
`object-close-brace` check the outdent of closing braces; `colors`,
`color-picker`, `color-accept` and `color-cancel` show the RGB vectors and
the shared colour picker. The full scenarios are in
[docs/TESTING.md](docs/TESTING.md).

The editor also offers `Ctrl+K` (or `Ctrl+/`) for line comments,
`Ctrl+Shift+K` for block comments, and Tab / Shift+Tab to indent or outdent
the selected lines. The arrows in the margin fold the blocks between braces;
the Edit menu offers Collapse all / Expand all. The `comment-lines`,
`comment-block`, `multiline-indent`, `multiline-outdent`, `folding` and
`object-folding` demos check these actions offline.

The inventory's right-click menu offers "Move to…" for a folder, an item or a
multiple selection. Type a name in the window's filter, choose the
destination folder then confirm with Move.

The full list of switches (demo scenarios, captures, forced settings,
diagnostics) and how to read the profiling output are in
[docs/TESTING.md](docs/TESTING.md).

Logs are in `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\`.

Conversation history (plain text, one file per conversation) is in
`%APPDATA%\Aurora\AuroraViewer\config\accounts\<account id>\chat_logs\<grid>\`;
Preferences › Chat shows the folder, turns logging off and erases it. These
files never leave the machine and are never attached to a bug report.

## Contributing

This repository is worked on by humans together with AI agents, each agent on
its own task in its own git worktree:

- [AGENTS.md](AGENTS.md) — rules for AI agents (testing, safety, PRs, reviews)
- [HUMANS.md](HUMANS.md) — how the repository works for humans
- [ARCHITECTURE.md](ARCHITECTURE.md) — scope, crates and file layout
- [TASKS.md](TASKS.md) — what is done, in progress and to do
- [docs/BRANDING.md](docs/BRANDING.md) — palette, logos, icons and UI rules
- [docs/TESTING.md](docs/TESTING.md) — test switches (demo scenarios,
  captures, diagnostics) and profiling

Behaviour follows Firestorm: its sources are expected next to the repository
(`../phoenix-firestorm`, cloned from
[FirestormViewer/phoenix-firestorm](https://github.com/FirestormViewer/phoenix-firestorm)).

## License

Aurora Viewer is free software under the
[GNU General Public License v3](LICENSE). Parts of it are ported
from the Second Life viewer and Firestorm (originally LGPL 2.1); see
[NOTICE.md](NOTICE.md) for details and third-party licenses.

Aurora Viewer is not affiliated with Linden Lab or the Firestorm project.
Second Life® is a trademark of Linden Research, Inc.
