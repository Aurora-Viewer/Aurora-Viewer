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
| Noto 3D emoji font (downloaded by `scripts/fetch-assets.ps1`) | `assets/emoji/` | SIL Open Font License 1.1 ([LICENSE-OFL.txt](assets/emoji/LICENSE-OFL.txt)) |
| Aurora wolf logo and silhouette | `assets/branding/` | Part of Aurora Viewer, GPL-3.0-or-later |
| LSL keyword names, signatures and help from Firestorm `indra/newview/app_settings/keywords_lsl_default.xml` | `crates/aurora-viewer/assets/lsl/keywords.json`, `reference.json` | Originally LGPL 2.1, distributed here under GPL-3.0-or-later as described above |
| LSL preprocessor rules from Modular Systems / Firestorm `indra/newview/fslslpreproc.cpp` | `crates/aurora-viewer/src/ui/scripts/preprocessor/` | BSD-3-Clause ([notice](crates/aurora-viewer/assets/lsl/PREPROCESSOR-LICENSE.txt)), retained alongside this GPL-3.0-or-later adaptation |

Fonts keep their own license (the OFL allows bundling them with software under
any license). The license texts of the fonts are next to them.

## Rust dependencies

The crates used by Aurora Viewer (see `Cargo.lock`) are under licenses
compatible with the GPL version 3 (mostly MIT and/or Apache-2.0).
