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
| Phosphor Icons — Copyright © 2020-2024 Phosphor Icons | `assets/phosphor-icons/`, `crates/aurora-viewer/assets/icons/phosphor/` | MIT ([LICENSE](assets/phosphor-icons/LICENSE)) |
| Inter typeface | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 |
| Noto Sans | `crates/aurora-viewer/assets/fonts/` | SIL Open Font License 1.1 |
| Roboto | `crates/aurora-viewer/assets/fonts/` | Apache License 2.0 |
| Noto 3D emoji font (downloaded by `scripts/fetch-assets.ps1`) | `assets/emoji/` | SIL Open Font License 1.1 ([LICENSE-OFL.txt](assets/emoji/LICENSE-OFL.txt)) |
| Aurora wolf logo and silhouette | `assets/branding/` | Part of Aurora Viewer, GPL-3.0-or-later |
| Firestorm / Second Life Sit, Buy/Pay, Open, Play, Pause, Open Media, Zoom and Grab cursors | `crates/aurora-viewer/assets/cursors/` | Original Firestorm artwork; source and conversion documented in [README](crates/aurora-viewer/assets/cursors/README.md), covered by the viewer source notice above |

Fonts keep their own license (the OFL allows bundling them with software under
any license). The license texts of the fonts are next to them.

## Rust dependencies

The crates used by Aurora Viewer (see `Cargo.lock`) are under licenses
compatible with the GPL version 3 (mostly MIT and/or Apache-2.0).
