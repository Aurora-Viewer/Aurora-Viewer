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
The `character/bake/` subdirectory contains the same sources' TGA skin,
clothing and visibility masks, used by the local wearable preview.
The Blender extension bundles the same `avatar_skeleton.xml` and retains its
provenance in `blender/aurora_live/SKELETON-NOTICE.txt`.

The terrain alpha ramp (`crates/aurora-render/src/terrain_alpha_ramp.r8`) is the
decoded 256-square linear grayscale image from the Legacy Viewer's
`indra/newview/skins/default/textures/alpha_gradient_2d.j2c` (source revision
`003e22b6`), Copyright Linden Research, Inc., originally LGPL 2.1 and distributed
here under GPL-3.0-or-later as described above. Rows are retained top-down.

The RLVa command parser, restriction rules, inventory conventions and camera
behaviors in `crates/aurora-viewer/src/world/rlva/` and `world/rlva.rs` follow
the Legacy Viewer's `indra/newview/rlv*.cpp` and `rlv*.h`: Copyright ©
2009-2020 Kitty Barnett, originally LGPL 2.1, distributed here under
GPL-3.0-or-later as described above.

## Bundled assets

| Asset | Location | License |
|---|---|---|
| Tabler Icons — Copyright © 2020-2026 Paweł Kuna | `assets/tabler-icons/`, `crates/aurora-viewer/assets/icons/tabler/` | MIT ([LICENSE](assets/tabler-icons/LICENSE)) |
| Inter typeface, Regular and SemiBold, version 4.001 | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-Inter.txt](crates/aurora-viewer/assets/fonts/LICENSE-Inter.txt)) |
| Noto Sans, Regular and SemiBold, version 2.015 (notofonts/latin-greek-cyrillic, hinted) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-NotoSans.txt](crates/aurora-viewer/assets/fonts/LICENSE-NotoSans.txt)) |
| Roboto, Regular and Medium, version 3.011 (googlefonts/roboto-3-classic, hinted) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-Roboto.txt](crates/aurora-viewer/assets/fonts/LICENSE-Roboto.txt)) |
| OpenDyslexic, Regular and Bold, version 0.910.12 — Copyright 2012-2019 Abbie Gonzalez, Reserved Font Name OpenDyslexic (antijingoist/opendyslexic, release `v0.91.12`, unmodified) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-OpenDyslexic.txt](crates/aurora-viewer/assets/fonts/LICENSE-OpenDyslexic.txt)) |
| Noto Color Emoji font, version 2.057 — Copyright 2022 Google Inc. (googlefonts/noto-emoji, tag `v2026-09-24-unicode18_0`, built into the executable) | `assets/emoji/` | SIL Open Font License 1.1 ([LICENSE-OFL.txt](assets/emoji/LICENSE-OFL.txt)) |
| Aurora wolf logo and silhouette | `assets/branding/` | Part of Aurora Viewer, GPL-3.0-or-later |
| Photo filters of the Second Life viewer `indra/newview/app_settings/filters` (the 30 XML files unchanged, from the Legacy Viewer's sources at commit `8ffe7065`), made by Linden Lab | `crates/aurora-render/src/looks/legacy/` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| LSL keyword names, signatures and help from Firestorm `indra/newview/app_settings/keywords_lsl_default.xml` | `crates/aurora-viewer/assets/lsl/keywords.json`, `reference.json` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Windlight presets of Firestorm `indra/newview/app_settings/windlight` (`days`, `skies`, `water`; XML files unchanged, commit `c33bfee2`), made by Linden Lab and by Second Life residents for the Phoenix / Firestorm viewers (their names and tags are in the preset names); the files carry no notice of their own and are distributed in the Firestorm sources under its LGPL 2.1 (`LICENSE`, `doc/LICENSE-source.txt`) | `crates/aurora-viewer/assets/windlight/windlight.zip` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Hunspell spelling dictionaries of ten languages (wooorm/dictionaries, commit `8cfea406`, unchanged; built into the executable, the list and the origin of each in [README.md](crates/aurora-viewer/assets/dictionaries/README.md)) | `crates/aurora-viewer/assets/dictionaries/` | Their own licences, each next to its dictionary (`LICENSE.txt`) and shipped in `assets/dictionaries/`: MIT AND BSD (English), MPL-2.0 (French), GPL-2.0 OR GPL-3.0 (German), GPL-3.0 OR LGPL-3.0 OR MPL-1.1 (Spanish), GPL-3.0 OR LGPL-3.0 OR MPL-2.0 (Polish), LGPL-3.0 OR MPL-2.0 (Portuguese), BSD-3-Clause (Russian), GPL-2.0 OR LGPL-2.1 OR MPL-1.1 (Danish) |
| Second Life glossary of the Legacy Viewer's dictionaries (`sl.dic`) and its auto-replace lists (`indra/newview/app_settings/autoreplace.xml`, unchanged) | `crates/aurora-viewer/assets/dictionaries/sl/`, `crates/aurora-viewer/assets/autoreplace.xml` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| LSL preprocessor rules from Modular Systems / Firestorm `indra/newview/fslslpreproc.cpp` | `crates/aurora-viewer/src/ui/scripts/preprocessor/` | BSD-3-Clause ([notice](crates/aurora-viewer/assets/lsl/PREPROCESSOR-LICENSE.txt)), retained alongside this GPL-3.0-or-later adaptation |
| Colour and dithering code of egui-wgpu's `egui.wgsl` — Copyright © Emil Ernerfeldt and the egui contributors | `crates/aurora-render/src/shaders/labels.wgsl` | MIT OR Apache-2.0, retained alongside this GPL-3.0-or-later adaptation |
| Screen space shadows of Bend Studio (`bend_sss_gpu.h`, `bend_sss_cpu.h`, as shipped in Days Gone) — Copyright © 2023 Sony Interactive Entertainment, ported to WGSL and Rust | `crates/aurora-render/src/shaders/contact_shadows.wgsl`, `crates/aurora-render/src/contact_shadows.rs` | Apache-2.0, notice retained at the top of `contact_shadows.wgsl` |
| Ground-truth ambient occlusion of Intel's XeGTAO (`XeGTAO.hlsli`, `vaGTAO.hlsl`, GameTechDev/XeGTAO), with its bent normals — Copyright © 2016-2021 Intel Corporation, ported to WGSL | `crates/aurora-render/src/shaders/gtao.wgsl`, `crates/aurora-render/src/gtao.rs` | MIT, notice retained at the top of `gtao.wgsl` |
| Spherical caps' intersection and cone specular occlusion of Google's Filament (`sphericalCapsIntersection`, `specularAO_Cones`, google/filament) — Copyright © 2018 The Android Open Source Project, rewritten in WGSL | `crates/aurora-render/src/shaders/common.wgsl` (`spherical_caps_intersection`, `cone_specular_occlusion`) | Apache-2.0, retained alongside this GPL-3.0-or-later adaptation |

Fonts keep their own license (the OFL allows bundling them with software under
any license). The license texts of the fonts are next to them.

## Media plugins

Web pages and video on media faces go through a separate pack, built by the
"Media plugins" workflow (`.github/workflows/media-plugins.yml`) and published
as the GitHub releases `media-plugins-<n>`. The viewer downloads it when media
need it and installs it in the user's data folder. Each release attaches the
license texts (`LICENSES\` in the archive) and the corresponding source of the
Linden Lab code. On Linux, video plays through the system's GStreamer, which
the pack does not include.

| Component | License |
|---|---|
| `SLPlugin.exe`, `media_plugin_cef.dll`, `media_plugin_libvlc.dll` (Windows), `SLPlugin`, `libmedia_plugin_cef.so`, `libmedia_plugin_gstreamer.so` (Linux), built unmodified from a pinned commit of the Firestorm sources | LGPL 2.1 (the commit is named in the pack's `aurora-media-plugins.json` and in the release notes) |
| Chromium Embedded Framework, Dullahan (Linden Lab's prebuilt package) | BSD-3-Clause and the licenses of Chromium's third-party code |
| LibVLC and its modules (Linden Lab's prebuilt package of VLC 3.0) | LGPL 2.1 or later; some modules GPL 2 or later |
| Microsoft Visual C++ runtime (`msvcp140.dll`, `vcruntime140*.dll`) | Redistributable under the Visual Studio license terms |

## Rust dependencies

The crates used by Aurora Viewer (see `Cargo.lock`) are under licenses
compatible with the GPL version 3 (mostly MIT and/or Apache-2.0).

## HIDAPI

The MIT-licensed `hidapi` Rust crate bundles native HIDAPI for SpaceMouse
input. That native code is distributed here under its BSD-style license
option. Source: https://github.com/libusb/hidapi (vendored by the crate
version recorded in `Cargo.lock`).

Copyright (c) 2010, Alan Ott, Signal 11 Software
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright notice,
      this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above copyright
      notice, this list of conditions and the following disclaimer in the
      documentation and/or other materials provided with the distribution.
    * Neither the name of Signal 11 Software nor the names of its
      contributors may be used to endorse or promote products derived from
      this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.

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
