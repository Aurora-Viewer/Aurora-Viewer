# Notices — Aurora Viewer

Copyright © 2026 the Aurora Viewer contributors (the authors in the project's
Git history).

Aurora Viewer is free software: you can redistribute it and/or modify it under
the terms of the **GNU General Public License, version 3 or (at your option) any
later version** (see [LICENSE](LICENSE)). A modified version must keep these
notices, including where the viewer shows them in its About window, and say
that it was modified (sections 4 and 5 of the licence).

The licence covers the code and the assets; it grants no right to the name
"Aurora Viewer" or to the Aurora logo as trademarks. What a modified version may
do with them is in [TRADEMARKS.md](TRADEMARKS.md).

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

## Authors of the ported code

The Second Life viewer and Firestorm sources hold code written by other
viewer projects and by individual residents, who keep the copyright on it.
The ported parts of Aurora Viewer that follow their files are listed below,
with the authors named in those files' notices. Their code is originally
LGPL 2.1, distributed here under GPL-3.0-or-later as described above.

| Original code (`indra/`) and its authors | Ported in (`crates/`) | Licence |
|---|---|---|
| Region tracker, `newview/alfloaterregiontracker.cpp` — Copyright © 2014 Alchemy Viewer Project | `aurora-viewer/src/world/region_tracker.rs`, `ui/region_tracker.rs` | Originally LGPL 2.1 |
| Animation overrider, `newview/aoengine.cpp`, `aoengine.h`, `aoset.cpp`, `ao.cpp` — Copyright © 2011 Zi Ree | `aurora-viewer/src/ao/`, `ui/ao/`, `movement.rs`, `movement/water_walk.rs`, `world/friend_join.rs` | Originally LGPL 2.1 |
| Animation explorer, `newview/animationexplorer.cpp` — Copyright © 2013 Zi Ree | `aurora-viewer/src/app/animation_explorer.rs`, `ui/animation_explorer.rs`, `world/animation_log.rs` | Originally LGPL 2.1 |
| Particle editor, `newview/particleeditor.cpp` — Copyright © 2011 Zi Ree | `aurora-viewer/src/app/particle_editor.rs`, `build/particle_editor.rs`, `build/ui/particles.rs`, `scene/particles.rs` | Originally LGPL 2.1 |
| Pie menu, `newview/piemenu.cpp` — Copyright © 2011 Zi Ree | `aurora-viewer/src/ui/context/pie.rs` | Originally LGPL 2.1 |
| Nearby chat, `newview/fsnearbychathub.cpp`, `fsfloaternearbychat.cpp` — Copyright © 2012 Zi Ree | `aurora-net/src/message_split.rs`, `session.rs`; `aurora-viewer/src/ui/chat.rs`, `ui/chat/`, `app/gestures.rs`, `world/bridge.rs`, `world/transcripts.rs`, `world/rlva/chat.rs` | Originally LGPL 2.1 |
| Texture tab, `newview/fspanelface.cpp` — Copyright © 2024 Zi Ree | `aurora-viewer/src/build/ui/texture.rs`, `build/ui/texture_drop.rs`, `build/contents_drop.rs` | Originally LGPL 2.1 |
| Object message filter, `newview/omnifilterengine.h` — Copyright © 2025 Zi Ree | `aurora-viewer/src/world/object_im_filter.rs` | Originally LGPL 2.1 |
| Quick preferences, `newview/quickprefs.cpp` — Copyright © 2011 WoLf Loonie, © 2013 Zi Ree, © 2013 Ansariel Hiller, © 2013 Cinder Biscuits | `aurora-viewer/src/ui/quick_prefs.rs`, `ui/environment.rs`, `world/eep_photo.rs`, `world/env_select.rs` | Originally LGPL 2.1 |
| Radar, `newview/fsradar.cpp`, `fsradarentry.cpp`, `fspanelradar.cpp` — Copyright © 2013 Ansariel Hiller; `fsradarlistctrl.cpp` — Copyright © 2011 Arrehn Oberlander | `aurora-net/src/bridge/query.rs`; `aurora-viewer/src/world/radar.rs`, `ui/people/radar.rs`, `ui/people/nearby.rs`, `world/friend_join.rs`, `app.rs` | Originally LGPL 2.1 |
| Money tracker, `newview/fsmoneytracker.cpp` — Copyright © 2011 Arrehn Oberlander, © 2015 Ansariel Hiller | `aurora-viewer/src/ui/money_tracker.rs` | Originally LGPL 2.1 |
| Asset blacklist, `newview/fsassetblacklist.cpp`, `fsfloaterassetblacklist.cpp` — Copyright © 2012 Wolfspirit Magic, © 2016 Ansariel Hiller | `aurora-viewer/src/world/derender.rs`, `ui/blacklist.rs` | Originally LGPL 2.1 |
| Common helpers, `newview/fscommon.cpp` — Copyright © 2012 Ansariel Hiller | `aurora-viewer/src/build/new_defaults.rs`, `ui/chat.rs`, `ui/emoji/recent.rs`, `world/groups.rs` | Originally LGPL 2.1 |
| Avatar render settings, `newview/fsfloateravatarrendersettings.cpp` — Copyright © 2016 Ansariel Hiller | `aurora-viewer/src/ui/avatar_render_settings.rs` | Originally LGPL 2.1 |
| Group titles, `newview/fsfloatergrouptitles.cpp` — Copyright © 2012 Ansariel Hiller | `aurora-viewer/src/ui/group_titles.rs` | Originally LGPL 2.1 |
| Place details, `newview/fsfloaterplacedetails.cpp` — Copyright © 2012 Ansariel Hiller | `aurora-viewer/src/ui/place_details.rs`, `ui/places.rs`, `world/place_details.rs` | Originally LGPL 2.1 |
| Partial inventory, `newview/fsfloaterpartialinventory.cpp` — Copyright © 2022 Ansariel Hiller | `aurora-viewer/src/ui/inventory.rs` | Originally LGPL 2.1 |
| Stream title, `newview/fsfloaterstreamtitle.cpp` — Copyright © 2022 Ansariel Hiller; `streamtitledisplay.cpp` — Copyright © 2010 Katharine Berry, © 2013 Cinder Roxley | `aurora-viewer/src/stream_title.rs`, `ui/stream_title.rs`, `app.rs` | Originally LGPL 2.1 |
| Desktop notifications, `newview/growlmanager.cpp` — Copyright © 2010 Katharine Berry | `aurora-viewer/src/system_notify.rs`, `app/attention.rs`, `world/attention.rs` | Originally LGPL 2.1 |
| Search, `newview/fsfloatersearch.cpp` — Copyright © 2012 Cinder Roxley | `aurora-net/src/search.rs`; `aurora-viewer/src/ui/search.rs`, `ui/search/details.rs`, `world/search.rs` | Originally LGPL 2.1 |
| Contact sets, `newview/lggcontactsets.cpp` — Copyright © 2011 Greg Hendrickson (LordGregGreg Back); `fspanelcontactsets.cpp`, `fsfloatercontactsetconfiguration.cpp` — Copyright © 2013 Cinder Roxley | `aurora-viewer/src/world/contact_sets.rs`, `world/contact_sets/`, `ui/contacts.rs`, `ui/people/sets.rs` | Originally LGPL 2.1 |
| Poser, `newview/fsfloaterposer.cpp`, `fsposeranimator.cpp`, `fsjointpose.h` — Copyright © 2024 Angeldark Raymaker | `aurora-viewer/src/ik_edit/poser.rs`, `ik_edit/poser/`, `scene/avatar_studio.rs` | Originally LGPL 2.1 |
| Area search, `newview/fsareasearch.cpp` — Copyright © 2012 Techwolf Lupindo | `aurora-viewer/src/app/area_search.rs`, `ui/area_search.rs`, `ui/area_search/`, `settings/area_search.rs`, `build/property_query.rs` | Originally LGPL 2.1 |
| Local mesh, `newview/vjlocalmesh.cpp` — Copyright © 2022 Vaalith Jinn | `aurora-viewer/src/blender_live/protocol.rs` | Originally LGPL 2.1 |
| Grids and SLURLs, `newview/fsgridhandler.cpp`, `fsslurl.cpp` — with modifications Copyright © 2012 arminweatherwax | `aurora-viewer/src/grids.rs`, `slurl.rs` | Originally LGPL 2.1 |
| Media lists, `newview/floatermedialists.cpp` — Copyright © 2011 Sione Lomu | `aurora-viewer/src/media/filter.rs` | Originally LGPL 2.1 |
| Simulator features, `newview/lfsimfeaturehandler.cpp` — Copyright © 2013 Liru Færs | `aurora-viewer/src/grid_status.rs` | Originally LGPL 2.1 |
| Interface sounds, `newview/fspanelpreferenceuisounds.cpp` — Copyright © 2026 Firestorm Viewer Project | `aurora-viewer/src/ui_sound.rs`, `ui/options/pages/sound.rs` | Originally LGPL 2.1 |
| Avatar alignment (Compass, Face nearest avatar), `newview/fsfloateravataralign.cpp` — Copyright © 2026 Ayane Lyla | `aurora-viewer/src/avatar_align.rs`, `app/avatar_align.rs`, `ui/avatar_align.rs` | Originally LGPL 2.1 |
| Calculator, `llmath/llcalc.cpp`, `llcalcparser.h` — Copyright © 2008 Aimee Walton | `aurora-viewer/src/cmdline/calc.rs` | Originally LGPL 2.1 |
| Script recovery, `newview/llfloaterscriptrecover.cpp` — Copyright © 2011-2012 Kitty Barnett | `aurora-net/src/session/asset_imports.rs`; `aurora-viewer/src/app/script_recovery.rs`, `ui/scripts/recovery.rs` | Originally LGPL 2.1 |
| RLVa visual effects, `newview/rlveffects.cpp`, `app_settings/shaders/class1/deferred/rlvF.glsl` — Copyright © 2018-2021 Kitty Barnett | `aurora-render/src/rlva.rs`, `rlva/overlay.rs`, `shaders/rlva.wgsl`; `aurora-viewer/src/world/rlva/effects.rs` | Originally LGPL 2.1 |

## Bundled assets

| Asset | Location | License |
|---|---|---|
| Tabler Icons — Copyright © 2020-2026 Paweł Kuna | `assets/tabler-icons/`, `crates/aurora-viewer/assets/icons/tabler/` | MIT ([LICENSE](assets/tabler-icons/LICENSE)) |
| Inter typeface, Regular and SemiBold, version 4.001 | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-Inter.txt](crates/aurora-viewer/assets/fonts/LICENSE-Inter.txt)) |
| Noto Sans, Regular and SemiBold, version 2.015 (notofonts/latin-greek-cyrillic, hinted) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-NotoSans.txt](crates/aurora-viewer/assets/fonts/LICENSE-NotoSans.txt)) |
| Roboto, Regular and Medium, version 3.011 (googlefonts/roboto-3-classic, hinted) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-Roboto.txt](crates/aurora-viewer/assets/fonts/LICENSE-Roboto.txt)) |
| OpenDyslexic, Regular and Bold, version 0.910.12 — Copyright 2012-2019 Abbie Gonzalez, Reserved Font Name OpenDyslexic (antijingoist/opendyslexic, release `v0.91.12`, unmodified) | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 ([LICENSE-OpenDyslexic.txt](crates/aurora-viewer/assets/fonts/LICENSE-OpenDyslexic.txt)) |
| Noto Color Emoji font, version 2.057 — Copyright 2022 Google Inc. (googlefonts/noto-emoji, tag `v2026-09-24-unicode18_0`, built into the executable) | `assets/emoji/` | SIL Open Font License 1.1 ([LICENSE-OFL.txt](assets/emoji/LICENSE-OFL.txt)) |
| MIT KEMAR HRTF measurements, diffuse-field equalized set — Copyright 1994 MIT Media Laboratory, by Bill Gardner and Keith Martin (https://sound.media.mit.edu/resources/KEMAR.html: "provided free with no restrictions on use, provided the authors are cited"); turned into minimum-phase filters by `scripts/generate-hrtf.py` (built into the executable; origin, citation and changes in [README.md](assets/hrtf/README.md)) | `assets/hrtf/kemar.bin` | Free use with credit to the authors (their terms, quoted in the README) |
| Tycho-2 Catalogue — Høg E. et al. 2000 (A&A 355, L27; https://archive.eso.org/ASTROM/TYC-2/data/, CDS I/259); the naked-eye stars and a map of the fainter ones, made by `scripts/generate-star-catalog.py`, with V and B-V for its 50 brightest or colourless stars from the Hipparcos Catalogue (ESA 1997, ESA SP-1200) (built into the executable; origin and changes in [README.md](assets/stars/README.md)) | `assets/stars/stars.bin`, `assets/stars/milky_way.bin` | BSD-2-Clause, agreed by its lead author in 2015 ([LICENSE-TYCHO2.md](assets/stars/LICENSE-TYCHO2.md)) |
| Aurora wolf logo and silhouette | `assets/branding/` | Part of Aurora Viewer, GPL-3.0-or-later |
| Four modern native walk/run animations, Copyright Linden Research, Inc., unmodified from the Legacy Viewer's `indra/newview/app_settings/static_assets/` at revision `f9401764e4e98aa3800749044a152cb4dcf75437` (provenance in the directory's README) | `crates/aurora-viewer/assets/animations/locomotion/` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Photo filters of the Second Life viewer `indra/newview/app_settings/filters` (the 30 XML files unchanged, from the Legacy Viewer's sources at commit `8ffe7065`), made by Linden Lab | `crates/aurora-render/src/looks/legacy/` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Photo LUTs by Czar Fenris (20 looks made in Adobe Premiere Pro and DaVinci Resolve), packed by `scripts/generate-photo-luts.py` (built into the executable; origin and changes in [README.md](assets/photo-luts/README.md)) | `assets/photo-luts/czar-fenris.bin` | Shared with the viewer and its users by their creator, Czar Fenris, who remains their author (agreed on 2026-11-10) |
| Photo LUTs by Nanner Pudding (45 looks made in DaVinci Resolve), packed by `scripts/generate-photo-luts.py` (built into the executable; origin and changes in [README.md](assets/photo-luts/README.md)) | `assets/photo-luts/nanner-pudding.bin` | Shared with the viewer and its users by their creator, Nanner Pudding, who remains their author (agreed on 2026-11-10) |
| LSL keyword names, signatures and help from Firestorm `indra/newview/app_settings/keywords_lsl_default.xml` | `crates/aurora-viewer/assets/lsl/keywords.json`, `reference.json` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| SLua keyword names, signatures and help — Copyright (c) 2025 Linden Research, Inc. (secondlife/lsl-definitions, release `v0.6.12`, `generated/lua_keywords_pretty.xml` with the whitespace between tags removed; built into the executable) | `crates/aurora-viewer/assets/slua/keywords_lua_default.xml` | MIT ([LICENSE-lsl-definitions.txt](crates/aurora-viewer/assets/slua/LICENSE-lsl-definitions.txt)) |
| Windlight presets of Firestorm `indra/newview/app_settings/windlight` (`days`, `skies`, `water`; XML files unchanged, commit `c33bfee2`), made by Linden Lab and by Second Life residents for the Phoenix / Firestorm viewers (their names and tags are in the preset names); the files carry no notice of their own and are distributed in the Firestorm sources under its LGPL 2.1 (`LICENSE`, `doc/LICENSE-source.txt`) | `crates/aurora-viewer/assets/windlight/windlight.zip` (built into the executable) | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| Hunspell spelling dictionaries of ten languages (wooorm/dictionaries, commit `8cfea406`, unchanged; built into the executable, the list and the origin of each in [README.md](crates/aurora-viewer/assets/dictionaries/README.md)) | `crates/aurora-viewer/assets/dictionaries/` | Their own licences, each next to its dictionary (`LICENSE.txt`) and shipped in `assets/dictionaries/`: MIT AND BSD (English), MPL-2.0 (French), GPL-2.0 OR GPL-3.0 (German), GPL-3.0 OR LGPL-3.0 OR MPL-1.1 (Spanish), GPL-3.0 OR LGPL-3.0 OR MPL-2.0 (Polish), LGPL-3.0 OR MPL-2.0 (Portuguese), BSD-3-Clause (Russian), GPL-2.0 OR LGPL-2.1 OR MPL-1.1 (Danish) |
| Second Life glossary of the Legacy Viewer's dictionaries (`sl.dic`) and its auto-replace lists (`indra/newview/app_settings/autoreplace.xml`, unchanged) | `crates/aurora-viewer/assets/dictionaries/sl/`, `crates/aurora-viewer/assets/autoreplace.xml` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| LSL preprocessor rules from Modular Systems / Firestorm `indra/newview/fslslpreproc.cpp` | `crates/aurora-viewer/src/ui/scripts/preprocessor/` | BSD-3-Clause ([notice](crates/aurora-viewer/assets/lsl/PREPROCESSOR-LICENSE.txt)), retained alongside this GPL-3.0-or-later adaptation |
| Colour and dithering code of egui-wgpu's `egui.wgsl` — Copyright © Emil Ernerfeldt and the egui contributors | `crates/aurora-render/src/shaders/labels.wgsl` | MIT OR Apache-2.0, retained alongside this GPL-3.0-or-later adaptation |
| Screen space shadows of Bend Studio (`bend_sss_gpu.h`, `bend_sss_cpu.h`, as shipped in Days Gone) — Copyright © 2023 Sony Interactive Entertainment, ported to WGSL and Rust | `crates/aurora-render/src/shaders/contact_shadows.wgsl`, `crates/aurora-render/src/contact_shadows.rs` | Apache-2.0, notice retained at the top of `contact_shadows.wgsl` |
| Ground-truth ambient occlusion of Intel's XeGTAO (`XeGTAO.hlsli`, `vaGTAO.hlsl`, GameTechDev/XeGTAO), with its bent normals — Copyright © 2016-2021 Intel Corporation, ported to WGSL | `crates/aurora-render/src/shaders/gtao.wgsl`, `crates/aurora-render/src/gtao.rs` | MIT, notice retained at the top of `gtao.wgsl` |
| AMD FidelityFX Super Resolution 3.1, the upscaler (FidelityFX SDK 1.1.4, `sdk/include/FidelityFX/gpu/fsr3upscaler/` and the RCAS of `gpu/fsr1/ffx_fsr1.h`) — Copyright © 2024 Advanced Micro Devices, Inc., ported to WGSL and Rust | `crates/aurora-render/src/shaders/fsr/`, `crates/aurora-render/src/fsr.rs` | MIT ([LICENSE-FidelityFX.txt](crates/aurora-render/src/shaders/fsr/LICENSE-FidelityFX.txt)), retained alongside this GPL-3.0-or-later adaptation |
| Spherical caps' intersection and cone specular occlusion of Google's Filament (`sphericalCapsIntersection`, `specularAO_Cones`, google/filament) — Copyright © 2018 The Android Open Source Project, rewritten in WGSL | `crates/aurora-render/src/shaders/common.wgsl` (`spherical_caps_intersection`, `cone_specular_occlusion`) | Apache-2.0, retained alongside this GPL-3.0-or-later adaptation |
| "Ambiance Heavy Rain Loop" by DWOBoyle (https://freesound.org/people/DWOBoyle/sounds/136971/); changed: levelled, converted to 48 kHz stereo Ogg Vorbis (built into the executable; the changes in [README.md](assets/rain/README.md)) | `assets/rain/rain-heavy.ogg` | CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/) |
| "Light Rain" by Jeffreys2 (https://freesound.org/people/Jeffreys2/sounds/137022/) and "Rain Indoor Cabin (loop)" by PhilllChabbb (https://freesound.org/people/PhilllChabbb/sounds/232731/), cut, levelled and looped (built into the executable; changes in [README.md](assets/rain/README.md)) | `assets/rain/rain-light.ogg`, `assets/rain/rain-roof.ogg` | CC0 1.0 |
| "Three mighty thunders in row" by Nimlos (https://freesound.org/people/Nimlos/sounds/522025/), cut into five clips and cleaned (built into the executable; changes in [README.md](assets/rain/README.md)) | `assets/rain/thunder-*.ogg` | CC0 1.0 |
| "Snow 004" from ambientCG (https://ambientcg.com/view?id=Snow004), downscaled and packed (built into the executable; changes in [README.md](assets/snow/README.md)) | `assets/snow/snow-normal-height.png`, `assets/snow/snow-color-roughness.png` | CC0 1.0 |

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

The crates built into Aurora Viewer (see `Cargo.lock`) are under licenses
compatible with the GPL version 3 (mostly MIT and/or Apache-2.0).
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) lists each of them with its
license, and gives the license texts and the copyright notices their authors
ask to keep; the viewer shows it in Help › About, Libraries tab. It is
generated from `Cargo.lock` by `scripts/third-party-licenses.ps1`.

That list includes the native code some crates compile into the viewer, with
its own notices: OpenJPEG (`openjpeg-sys`, BSD-2-Clause), libvorbis with the
aoTuV tuning and libogg (`aotuv_lancer_vorbis_sys`, `ogg_next_sys`,
BSD-3-Clause), mimalloc (`libmimalloc-sys`, MIT), AWS-LC and ring, the TLS
cryptography (`aws-lc-sys`, `ring`: ISC, Apache-2.0 and the OpenSSL and
BoringSSL notices they carry), the OpenXR loader (`openxr-sys`, Apache-2.0) and
egui's default fonts (`epaint_default_fonts`: Ubuntu Font Licence 1.0 and SIL
Open Font License 1.1).

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

## AMD FidelityFX CAS

The world upscale in `crates/aurora-render/src/shaders/rlva_resolution.wgsl`
is derived from FidelityFX CAS's `CasFilter` scaling path:
<https://github.com/GPUOpen-Effects/FidelityFX-CAS/blob/master/ffx-cas/ffx_cas.h>.
The sharpening of `crates/aurora-render/src/shaders/post.wgsl` follows the
same filter's no-scaling path as the Legacy Viewer ships it
(`indra/newview/app_settings/shaders/class1/deferred/CASF.glsl`), whose
notice adds: Copyright (c) 2021 Advanced Micro Devices, Inc.; Copyright (c)
2014 Michal Drobot (for concepts used in "FLOAT APPROXIMATIONS").

Copyright (c) 2017-2019 Advanced Micro Devices, Inc. All rights reserved.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

## Vulkan glTF PBR by Sascha Willems

The physically based shading of `crates/aurora-render/src/shaders/common.wgsl`,
`env.wgsl`, `object.wgsl` and the prefiltered radiance of `probe_filter.wgsl`
follow the Legacy Viewer's `deferredUtil.glsl` and `radianceGenF.glsl`
(`indra/newview/app_settings/shaders/`), parts of which are taken from Sascha
Willems' Vulkan glTF reference implementation
(<https://github.com/SaschaWillems/Vulkan-glTF-PBR>), MIT License:

Copyright (c) 2018 Sascha Willems

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## Triplanar mapping by Inigo Quilez

The triplanar mapping of the PBR terrain in
`crates/aurora-render/src/shaders/env.wgsl` follows the Legacy Viewer's
`pbrterrainUtilF.glsl`, adapted from Inigo Quilez' example shader
(<https://www.shadertoy.com/view/MtsGWH>), MIT License:

Copyright © 2015 Inigo Quilez

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

## SMAA

The anti-aliasing of `crates/aurora-render/src/shaders/smaa.wgsl` is a WGSL port
of SMAA (<https://www.iryoku.com/smaa/>) as the Legacy Viewer ships it, and
`crates/aurora-render/src/smaa/area.bin` and `search.bin` are its precomputed
lookup textures. The full notice is in
[LICENSE-SMAA.txt](crates/aurora-render/src/smaa/LICENSE-SMAA.txt):

Copyright (C) 2013 Jorge Jimenez, Jose I. Echevarria, Belen Masia, Fernando
Navarro and Diego Gutierrez. MIT License (its authors state that the notice
need not be included in binary distributions; it is kept here all the same).
