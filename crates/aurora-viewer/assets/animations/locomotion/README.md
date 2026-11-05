# Native walk and run animations

These four unmodified Second Life animation assets come from the Legacy
Viewer's `indra/newview/app_settings/static_assets/`, source revision
`f9401764e4e98aa3800749044a152cb4dcf75437`:

| File UUID | Animation |
|---|---|
| `33339176-7ddc-9397-94a4-bf3403cbc8f5` | Modern male walk |
| `d60c41d2-7c24-7074-d3fa-6101cea22a51` | Modern female walk |
| `1ab1b236-cd08-21e6-0cbc-0d923fc6eca2` | Modern male run |
| `85995026-eade-5d78-d364-94a64512cb66` | Modern female run |

Copyright Linden Research, Inc. Originally distributed under LGPL 2.1
(`LICENSE` and `doc/LICENSE-source.txt` in the source distribution), included
here under GPL-3.0-or-later as described in the repository's `NOTICE.md`.

`scene/default_motions.rs` embeds and registers the clips at startup, following
the reference viewer's static asset cache. Standard walk/run signals resolve
to these assets only for avatar playback; their original network UUIDs and
all HUD animations are preserved. The clips key the pelvis and legs at
priority zero, allowing ordinary low-priority HUD AO animations to win.
