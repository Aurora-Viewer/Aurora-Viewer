# Folderstorm features to port

The features of Folderstorm (C++) (DrCum/Folderstorm, branch `main` at
`abbe3cb0ca`, 2026-11-05) to bring to Folderstorm2. Compiled from its
`README.md`, `doc/help.md`, `doc/plans/`, `doc/*.md` and its merged branches.
**Draft for the human to edit**: reorder, cut, merge or add lines freely.

Status: ⬜ to do · 🔧 in progress · 🟡 done, waiting for grid test · ✅ done ·
➖ not porting.

"Aurora today" says what upstream already has, where known; "check" means
nobody has looked yet. Priority is a first guess (P1 = the reasons the fork
exists, P2 = strong extras, P3 = later), for the human to change.

Source docs are paths in the Folderstorm (C++) repository.

---

## 0. Fork foundations

Needed before shipping any build. They also keep merges simple.

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 0.1 | Own name and icon | The viewer is called **Folderstorm2**. Window title, icon, About, settings folder, crash reports and installer use it. Logo for now: `folderstorm/branding/folderstorm2-logo.svg` (the winged folder with eyes; edits to come). Login screen with the winged folder, splash art, purple Log In, "I'm Sodie" feed. Required: Aurora's `TRADEMARKS.md` does not allow shipping a modified build as "Aurora Viewer" or with the wolf logo. Keep the GPL notices and say "based on Aurora Viewer". | Aurora branding throughout | `doc/help.md` "Login and identity" | P1 | ⬜ |
| 0.2 | CI for a fork | Disable or retarget the upstream-only workflows (nightly, fuzz, PR-description and review-slot bots, release to the Aurora org); keep fmt / clippy / tests. | Upstream workflows run on the fork | [CI.md](CI.md) | P1 | 🟡 workflow file added; the human enables Actions and disables the upstream-only workflows (CI.md) |
| 0.3 | Release checks and update feed | Check for updates against Folderstorm's own releases (Stable / Nightly), with highlights, Download, Skip this version. | `aurora-updater` crate, pointed at Aurora | `doc/plans/releases-crashes-and-updates.md`, `release-checks-validation.md`, `coordinated-updates-validation.md` | P2 | ⬜ |
| 0.4 | Crash reports | Send crash reports to Folderstorm's own service (Folderstorm (C++) used Sentry), never to Aurora's. | `crash` module, Aurora's endpoint | `doc/plans/sentry-validation.md` | P2 | ⬜ |
| 0.5 | Settings import | Optional copy of settings from another viewer on first run. Folderstorm (C++) had a `migrate-settings` tool for Firestorm → Folderstorm. Decide: import from Folderstorm (C++), from Aurora, or both. | check (Aurora has some import of Legacy settings) | `doc/help.md` "Settings from Firestorm" | P3 | ⬜ |

## 1. Wide-screen layout and workspaces (P1: why the fork exists)

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 1.1 | Custom world viewport | Preferences → Move & View → Layout. Stretch the window over several monitors and keep the 3D world (camera, HUD, mouse) in one rectangle set by left / right / top / bottom margins; the rest holds windows. Live preview; separate switch for mouselook; margins scaled down if less than 5% would remain; login and 2FA use the same slice. | check | `README.md` "World viewport", `doc/help.md` "Wide-screen layout" | P1 | ⬜ |
| 1.2 | Visual viewport setup | Interactive diagram of the window and monitors; pick a monitor to place the viewport, keep the numeric margins; handles unequal monitors, DPI, negative desktop coordinates. | – | `doc/plans/visual-viewport-setup.md` | P1 | ⬜ |
| 1.3 | Layout profiles | Place toolbars, chat bar and top bars around the viewport (outside when there is room, inside otherwise); save as named profiles; apply, rename, delete, Reset. | check | `doc/help.md` "Layout" | P1 | ⬜ |
| 1.4 | Workspaces | Per-account saved window arrangements: viewport and bars, Inventory windows (up to 16 extra), map, chat geometry, Received Items size. Preview, OK keeps, Cancel restores. Optional "Remember Inventory folders" (folder root and view per Inventory window). Never stores messages or inventory contents. Restored manually, not at login. | check | `doc/help.md` "Workspaces", `doc/plans/complete-workspaces.md`, `workspace-enhancements.md` | P1 | ⬜ |
| 1.5 | Workspace switcher | World → Workspaces & Layouts…, or a toolbar button. Type to filter, Enter to switch, favorites first, Preview…, Save new with a choice of what to capture. Keyboard switching, Previous arrangement, snapping and alignment, toolbar sets, export / import. | – | `doc/help.md`, `doc/plans/workspace-enhancements.md` | P1 | ⬜ |
| 1.6 | Workspace favorites strip | Optional row under the navigation / landmark bars with favorite workspaces and layouts, overflow in More…; hidden in mouselook. | – | `doc/help.md` | P2 | ⬜ |
| 1.7 | Workspace context | A workspace can also carry chosen graphics / camera settings and a set of worn HUDs (by inventory id); recovers when a monitor is missing. | – | `doc/plans/workspace-context-and-snapshot.md` | P2 | ⬜ |
| 1.8 | Workspace backgrounds | When the window is stretched over several monitors, show a chosen wallpaper in the area outside the world viewport instead of black (Preferences → Move & View → Backgrounds). Optional. | – | branch `feat/native-workspace-backgrounds` | P3 | ⬜ |
| 1.9 | Settings search keywords | Preferences search finds "MCP", "dual monitor", "workspace", "undo". | check (Aurora has a settings search) | `doc/help.md` | P3 | ⬜ |

## 2. Inventory management (P1: the other reason)

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 2.1 | Custom Inventory tabs | "Customize tab…" beside the search box edits a custom tab's filters; the tab remembers filters and search; right-click a tab for Customize, Rename, Clone. | check | `doc/help.md` "Custom Inventory tabs" | P1 | ⬜ |
| 2.2 | Folder-scoped saved tabs | Save an Inventory tab scoped to chosen folders. | – | branch `codex/inventory-view-folders` | P2 | ⬜ |
| 2.3 | Jump to folder | Go to an Inventory folder by typing its name or parent path. | check | branch `codex/inventory-folder-jump` | P1 | ⬜ |
| 2.4 | Bulk review (rename / move) | Select 1–50 items → gear → Review bulk changes…: preview a rename pattern or a move; choose the destination by searching folder or parent name; exact before / after rows and skipped items before running. | – | `doc/help.md` "Review Inventory changes", `doc/local-assistant-bulk-inventory.md`, `doc/plans/bulk-inventory-review-and-history.md` | P1 | ⬜ |
| 2.5 | Inventory change history and undo | This session's tracked rename / move operations; Stop, Clear history; undo only confirmed changes that still match their recorded state. | – | same as 2.4 | P2 | ⬜ |
| 2.6 | Local folder archive | Archive Inventory folders locally (hide them from the normal view) without moving their contents on the server. | – | branch `codex/inventory-archive` | P2 | ⬜ |
| 2.7 | Outfit safety | Context-menu Wear disabled by default (option to enable); folder Add To / Replace Current Outfit always ask (Cancel by default, no "don't ask again"). | check | `doc/inventory-outfit-safety.md` | P2 | ⬜ |
| 2.8 | Restore previous look | After replacing the outfit with a folder, one action restores the look worn just before. | – | branch `codex/previous-look` | P2 | ⬜ |
| 2.9 | Outfit Gallery thumbnail size | Slider for photo width, 80–256 px. | check | `doc/help.md` "Outfit Gallery" | P3 | ⬜ |
| 2.10 | Landmark folders on the favorites bar | Group favorite landmarks in local folders on the favorites bar, with drag and drop. | check | branch `codex/landmark-folders` | P2 | ⬜ |
| 2.11 | Folder-preserving bulk upload | Upload a folder of files and recreate its folder tree in Inventory. | check | branch `codex/sl-folder-upload` | P3 | ⬜ |
| 2.12 | Safe link replacement | Replace links (e.g. after a re-delivery): create the new link first, only then move the old one; report each item's result. | – | `doc/plans/safe-link-replacement.md` | P3 | ⬜ |

## 3. Chat and gestures

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 3.1 | Gesture boards | World → Gesture board…: tiles of gestures and sounds, dragged from Inventory; click to play; several saved, reorderable boards; editable tile text and colours; Options → Find tiles… (Ctrl+F). | check | `doc/help.md` "Gesture boards", `doc/plans/gesture-board.md` | P2 | ⬜ |
| 3.2 | Inline chat image previews | Image links (direct, GIF, sharing sites) show a preview under the message in Nearby, IM, group and conference chat; click to enlarge; play / pause for animations; option "Hide image links after loading". | check | `doc/plans/chat-media-previews.md` | P2 | ⬜ |
| 3.3 | Chat image upload | Paste, drop or choose an image in a chat composer; it uploads to Folderstorm's image host and puts the link in the draft (sending stays manual). Needs the image host (`images.sodie.online`, Zipline). | – | `doc/plans/native-chat-image-uploads.md` | P3 | ⬜ |
| 3.4 | GIF board | GIF button beside the composer: picker with KLIPY Search / Trending / Favourites / Recent and "My GIFs", pop-out board window. Needs a KLIPY API key, which must never be committed (decide how builds get it). | – | `doc/plans/folderstorm-gif-board-klipy.md`, `gif-board-validation.md`, `klipy-validation.md` | P3 | ⬜ |
| 3.5 | Squirty Notes | Desktop pop-ups for incoming IMs and friends coming online, through the Squirty Notes Windows app's local API (token, port 47831; Off / All / Custom lists; "squirt again after silence"). | – | `doc/squirty-notes.md` | P3 | ⬜ |

## 4. Local assistant (MCP)

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 4.1 | Local assistant bridge | Off by default. Preferences → Local assistant: in-viewer bridge on 127.0.0.1 with a discovery file and bearer token; permission presets Read only / Ask before changes / Custom, "this session only"; Ask dialogs (60 s); permanent delete and empty Trash always refused. `--mcp-api` forces it on for one session. | check: Aurora has a "scripting bridge" | `README.md`, `doc/help.md` "The local assistant bridge", `doc/fs-mcp.md`, `doc/plans/assistant-*.md` | P1 | ⬜ |
| 4.2 | `fs-mcp` sidecar | MCP server over stdio (Go, `tools/fs-mcp`) that finds the viewer and calls the bridge; ships beside the viewer. Could stay in Go or be rewritten in Rust as a separate crate. | – | `tools/fs-mcp/README.md` | P1 | ⬜ |
| 4.3 | Inventory and outfit tools | Search, create folders, move, rename, copy, Trash and restore; list outfits, wear, detach; wait for the outfit to finish baking. | – | `doc/help.md` "Asking for something" | P1 | ⬜ |
| 4.4 | Camera and snapshot tools | Camera presets (portrait, full body, front, back, left, right) and exact shots; snapshot of the window or the world viewport, sized and cropped; set it as an outfit / folder thumbnail (free) or upload as a texture (asks with the L$ price). | – | `doc/help.md` | P1 | ⬜ |
| 4.5 | Setup page | Copy configuration for Codex, Cursor or Claude Code; Check connection; `fs-mcp --diagnose`. | – | `doc/help.md` "Copy configuration and check this viewer" | P2 | ⬜ |
| 4.6 | Extended MCP tools | Navigation, world observation, object inspection and lifecycle, building, content editing, file uploads, gestures and posing, preferences, social and events, script testing, task scripts, session routing (multi-character). Port one group at a time once 4.1–4.4 work. | – | `doc/plans/mcp-*.md` | P3 | ⬜ |

## 5. Multi-character

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 5.1 | Session host | Separate Windows app that runs up to five characters, one viewer process each, in one interface: warm / economy standby, switching the active character without relogging, shared chat for all characters, low-rate read-only monitors of the others, pop-out panels, restart and recovery, character quick-action buttons, profile copy, audio / voice follows the active character. Off by default. The largest feature by far: re-plan it for Aurora rather than port it as is. | – | `doc/plans/multi-character-*.md`, `session-*.md`, `host-*.md`, `character-quick-actions.md`, `tools/session-host/` | P3 | ⬜ |

## 6. Small extras

| # | Feature | What it does | Aurora today | Source | Prio | Status |
|---|---|---|---|---|---|---|
| 6.1 | About: location choice | Choose whether the About report includes your in-world location. | check | branch `codex/about-location-choice` | P3 | ⬜ |
| 6.2 | First-run notices | Installation notices shown one at a time on first run. | check | branch `codex/first-run-notices` | P3 | ⬜ |

## Not porting (C++-specific or upstream fixes)

Listed so that nobody looks for them again.

- Animation, skinning and frame-loop performance work (`codex/perf-*`
  branches): specific to the C++ renderer. Aurora has its own.
- Fixes to the C++ code base: task inventory (`codex/task-inventory-*`),
  background inventory cache load (`codex/sl-inventory-cache`), startup
  prompts, the `upstream/20261101-*` sync branches.
- The C++ build, NSIS / Velopack installers and the Windows MCP link cleanup
  utility: Aurora has its own packaging. Revisit with 0.3 and 4.2.
- RLVa and script dialog changes: they came from the Legacy Viewer, and
  Aurora has its own versions.
- Shared poser and the Folderstorm colour themes (Folderstorm Violet,
  Electric Jungle): dropped by the human on 2026-11-10.
- Unmerged experiments (`codex/ansastorm-*` theme editor, `feat/next-improvements`,
  `Texture_VRAM_Optimizations`, …): ask the human before including any.
