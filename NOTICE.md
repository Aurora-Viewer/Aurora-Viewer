# Notices — Aurora Viewer

Aurora Viewer is free software: you can redistribute it and/or modify it under
the terms of the **GNU General Public License, version 3 or (at your option) any
later version** (see [LICENSE](LICENSE)).

Aurora Viewer is not provided, endorsed or supported by Linden Research, Inc.
("Linden Lab") or The Phoenix Firestorm Project, Inc. Second Life® is a
trademark of Linden Research, Inc.

## Code derived from the Second Life viewer and Firestorm

Many parts of this program are Rust ports of the Second Life viewer
(Copyright © Linden Research, Inc.) and of the Firestorm viewer
(Copyright © The Phoenix Firestorm Project, Inc.), both distributed under the
GNU Lesser General Public License, version 2.1. Each ported file says which
original sources it follows ("originally LGPL 2.1").

As permitted by section 3 of the LGPL 2.1, the copy of that code included in
this program is distributed under the GNU General Public License, version 3 or
any later version. The original code remains available under the LGPL 2.1 from
its authors:

- Second Life viewer — <https://github.com/secondlife/viewer>
- Firestorm — <https://github.com/FirestormViewer/phoenix-firestorm>

The Second Life message template (`crates/aurora-msg/message_template.msg`) and
the avatar definition files (`crates/aurora-viewer/assets/character/`) come from
the Second Life viewer sources (originally LGPL 2.1).

## Bundled assets

| Asset | Location | License |
|---|---|---|
| Tabler Icons — Copyright © 2020-2026 Paweł Kuna | `assets/tabler-icons/`, `crates/aurora-viewer/assets/icons/tabler/` | MIT ([LICENSE](assets/tabler-icons/LICENSE)) |
| Inter typeface | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 |
| Noto Sans | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 |
| Roboto | `crates/aurora-viewer/assets/fonts/` | Apache License 2.0 |
| Noto Color Emoji font, version 2.057 — Copyright 2022 Google Inc. (googlefonts/noto-emoji, tag `v2026-09-24-unicode18_0`, built into the executable) | `assets/emoji/` | SIL Open Font License 1.1 ([LICENSE-OFL.txt](assets/emoji/LICENSE-OFL.txt)) |
| Aurora wolf logo and silhouette | `assets/branding/` | Part of Aurora Viewer, GPL-3.0-or-later |
| LSL keyword names, signatures and help from Firestorm `indra/newview/app_settings/keywords_lsl_default.xml` | `crates/aurora-viewer/assets/lsl/keywords.json`, `reference.json` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Windlight presets of Firestorm `indra/newview/app_settings/windlight` (`days`, `skies`, `water`; XML files unchanged, commit `c33bfee2`), made by Linden Lab and by Second Life residents for the Phoenix / Firestorm viewers (their names and tags are in the preset names); the files carry no notice of their own and are distributed in the Firestorm sources under its LGPL 2.1 (`LICENSE`, `doc/LICENSE-source.txt`) | `crates/aurora-viewer/assets/windlight/windlight.zip` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| LSL preprocessor rules from Modular Systems / Firestorm `indra/newview/fslslpreproc.cpp` | `crates/aurora-viewer/src/ui/scripts/preprocessor/` | BSD-3-Clause ([notice](crates/aurora-viewer/assets/lsl/PREPROCESSOR-LICENSE.txt)), retained alongside this GPL-3.0-or-later adaptation |
| Colour and dithering code of egui-wgpu's `egui.wgsl` — Copyright © Emil Ernerfeldt and the egui contributors | `crates/aurora-render/src/shaders/labels.wgsl` | MIT OR Apache-2.0, retained alongside this GPL-3.0-or-later adaptation |

Fonts keep their own license (the OFL allows bundling them with software under
any license). The license texts of the fonts are next to them.

## Media plugins

Web pages and video on media faces go through a separate pack, built by the
"Media plugins" workflow (`.github/workflows/media-plugins.yml`) and published
as the GitHub releases `media-plugins-<n>`. The viewer downloads it when media
need it and installs it in the user's data folder. Each release attaches the
license texts (`LICENSES\` in the archive) and the corresponding source of the
Linden Lab code.

| Component | License |
|---|---|
| `SLPlugin.exe`, `media_plugin_cef.dll`, `media_plugin_libvlc.dll`, built unmodified from a pinned commit of the Firestorm sources | LGPL 2.1 (the commit is named in the pack's `aurora-media-plugins.json` and in the release notes) |
| Chromium Embedded Framework, Dullahan (Linden Lab's prebuilt package) | BSD-3-Clause and the licenses of Chromium's third-party code |
| LibVLC and its modules (Linden Lab's prebuilt package of VLC 3.0) | LGPL 2.1 or later; some modules GPL 2 or later |
| Microsoft Visual C++ runtime (`msvcp140.dll`, `vcruntime140*.dll`) | Redistributable under the Visual Studio license terms |

## Rust dependencies

The crates used by Aurora Viewer (see `Cargo.lock`) are under licenses
compatible with the GPL version 3 (mostly MIT and/or Apache-2.0).

## V-HACD

The unmodified V-HACD 4.1 header in `crates/aurora-assets/native/vhacd/`
is distributed under BSD-3-Clause. Its source revision and checksum are
recorded alongside it. The following notice also applies to its compiled
code included in Aurora Viewer:

BSD 3-Clause License

Copyright (c) 2011, Khaled Mamou (kmamou at gmail dot com)
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

## Sonora audio processing

Sonora 0.2 and its sonora-* components provide the pure Rust WebRTC audio
processing pipeline (AEC3, noise suppression, AGC2 and high-pass filtering).
Source: https://github.com/dignifiedquire/sonora (BSD-3-Clause).

Copyright (c) 2011, The WebRTC Project Authors. All rights reserved.
Copyright (c) 2016, Arun Raghavan and contributors. All rights reserved.
Copyright (c) 2026, dignifiedquire. All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

  * Redistributions of source code must retain the above copyright
    notice, this list of conditions and the following disclaimer.

  * Redistributions in binary form must reproduce the above copyright
    notice, this list of conditions and the following disclaimer in
    the documentation and/or other materials provided with the
    distribution.

  * Neither the name of Google nor the names of its contributors may
    be used to endorse or promote products derived from this software
    without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
