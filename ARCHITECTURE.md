# Aurora Viewer architecture

## Scope

Aurora Viewer is a complete Second Life viewer written in Rust: login,
UDP and HTTP networking, world (objects, terrain, avatars, environment),
modern rendering (wgpu on Vulkan), sound, voice, media and interface (egui).

**Reference behaviour: Firestorm.** Every feature reproduces what Firestorm
does (and the Linden Lab viewer it derives from): same network messages,
same rules, same defaults, unless an explicit choice is noted in `TASKS.md`.
The interface, for its part, is Aurora's own (see
[docs/BRANDING.md](docs/BRANDING.md)). Firestorm is the reference for the
result, not for the implementation: the rendering must be the same or
better, but the engine is designed for Vulkan and the modern GPU, without
taking over the techniques inherited from OpenGL (see
[AGENTS.md](AGENTS.md#2-reference-behaviour-firestorm)).

**Out of scope for now:** OpenSim grids, support for platforms other than
Windows for the releases (the code stays portable).

## File tree

The background of the update and exit dialogs is blurred by
`aurora-render/src/ui_blur.rs` and `shaders/ui_blur.wgsl`: an egui marker
separates the background layers from the dialog. The scene and these layers
are rendered into a texture, blurred horizontally then vertically on the
GPU; the dialog stays sharp. These targets exist only while the dialog is
displayed.

```
aurora-viewer/                  git repository (GitHub: Aurora-Viewer/Aurora-Viewer)
├─ Cargo.toml                   workspace: version, edition, licence, common lints
├─ rust-toolchain.toml          stable Rust (always the latest version)
├─ rustfmt.toml                 formatting (140-character lines)
├─ README.md                    overview, differences from Firestorm, build
├─ AGENTS.md                    rules for AI agents (Claude, Codex…)
├─ CLAUDE.md                    points to AGENTS.md
├─ HUMANS.md                    guide for humans: PRs, merge, build
├─ ARCHITECTURE.md              this file
├─ TASKS.md                     task tracking: in progress / to do / finished since the last release (older: docs/TASKS_ARCHIVE.md)
├─ LICENSE, NOTICE.md           GPLv3, third-party licences
├─ aurora-tools.cmd             development tools (double-click): scripts/tools
├─ assets/
│  ├─ branding/                 logos (colour, silhouette), SVG + PNG 4096, aurora.ico
│  ├─ models/                   self-contained Aurora 3D GLB for material previews
│  ├─ tabler-icons/             full Tabler library (SVG: outline, filled)
│  └─ emoji/                    emoji font (downloaded, outside git; inside the exe of a published build)
├─ crates/                      the code (see below)
├─ docs/                        BRANDING.md, TESTING.md (`AURORA_*` test options, profiling), README screenshots (`screenshots/`), README buttons and "Soon" pill (`readme/`)
├─ news/                        news of the login screen, written by hand: fr.md, en.md (read on `main` by the viewer, and compiled into it)
├─ scripts/                     tools for agents and humans (PowerShell)
│  ├─ automation-window-border.ps1 dashed yellow outline of the automated-test windows (Windows)
│  └─ tools/                    aurora-tools: menu, interface (ui), checks and repair (checks), actions, live PRs (prs), releases, reset (reset), Claude Code permissions (claude)
└─ .github/                     CI and wiki publication (workflows/, actions/), release, PR template, Dependabot
```

Next to the repository (outside git), in the project folder installed by the
one-line command (`scripts/setup.ps1`, then the tools):

```
<project folder>\               for example C:\Aurora-Viewer
├─ aurora-viewer\               the repository
├─ phoenix-firestorm\           official Firestorm sources (read-only)
├─ RELEASE\                     --release build of the latest main (scripts/build-release.ps1)
└─ work\                        one git worktree per agent task
```

## The crates

| Crate | Role |
|---|---|
| `aurora-viewer` | The application: window loop (winit), world state, scene, egui interface, agent, camera, offline demo |
| `aurora-updater` | Stable GitHub releases and maintenance thresholds; cancellable download, SHA-256 and bounded ZIP extraction; Windows helper, transactional replacement and rollback until the new version has presented a frame. `user_data.rs` backs up/restores the cache and `settings.json` if the policy requires it. `media_pack.rs` installs the media plugins pack (release `media-plugins-<n>`, size and SHA-256 pinned in the viewer) in the user's data folder, with the same download and extraction. Isolated HTTP and file tests. |
| `aurora-version` | Common source of the compiled version: Git tag, provenance of builds between tags, HTTP User-Agent and SL login version with four numeric components. Its `build.rs` also tracks the Git references of the worktrees and passes the version to the viewer's `build.rs` (`links`), for the version block of the exe; `build_support.rs` holds the tested pure resolution. |
| `aurora-i18n` | Language shared by the viewer and the network messages: Fluent catalogues `locales/*.ftl`, OS detection, saved choice, English fallback, variables, plurals and decimals. `build.rs` discovers the languages automatically; simple labels and arrays of labels are cached per language. See `docs/TRANSLATING.md`. |
| `aurora-net` | Login (XML-RPC), UDP circuits, capabilities, event queue, HTTP download of assets; runs on its own tokio runtime. `agent_update.rs`: when an AgentUpdate leaves (a change of the control flags at once and confirmed, the rest past Firestorm's thresholds at 125 a second at most, one a second to stay alive); the viewer hands the session what must leave before its next tick (`NetClient::set_controls`). `throttle.rs`: the bandwidth asked from the simulator, port of LLViewerThrottle (the setting shared between the seven channels, lowered while packets are lost or the viewer falls behind). `objcache.rs`, `session/object_cache.rs`: per-region object cache (the viewer side of ObjectUpdateCached) and its files kept off the session loop: read by a blocking task before RegionHandshakeReply leaves, written from snapshots by one writer. `session/object_actions.rs`: touch / grab / release and object inventory (capability or memory-bounded UDP transfer); `task_inventory.rs`: LLSD parsing and legacy file; `land.rs`, `session_land.rs`: parcel messages ("About Land", RemoteParcelRequest of any place) |
| `aurora-msg` | SL's UDP message system: packets, zerocoding, typed messages generated from `message_template.msg` (build.rs) |
| `aurora-llsd` | The LLSD type and its XML / binary / notation formats |
| `aurora-prim` | Prim model: volume parameters, faces, extended parameters, geometry generation (port of `llvolume`) |
| `aurora-assets` | Asset decoders: JPEG2000, mesh, animations, materials, `.llm` avatar meshes, skeleton |
| `aurora-render` | wgpu / Vulkan rendering engine: bindless textures grouped into pages (texture arrays), sub-allocated geometry, multi-draw-indirect, shadows, reflections, post-processing; texture animations (`tex_anim.rs`: CPU reference and parameters of the records, evaluated by the vertex shaders); GPU-driven draw lists (`gpu_cull.rs`, `shaders/cull.wgsl`: face and object tables maintained by the scene, culling and compaction in compute for each view, drawing through `multi_draw_indexed_indirect_count`); local lights sorted by screen tile and by depth (`light_tiles.rs`: for each column, row and depth slice of the main view, the set of lights that cover it, computed every frame on the render thread; a fragment only walks the lights common to the three, `light_walk` in `shaders/common.wgsl`, instead of the 64 of the list; the reflection probes of a fragment are found the same way, in sets of their own, `probes::tiles` and `probe_tiles`); two-phase Hi-Z occlusion (`occlusion.rs`, `shaders/occlusion.wgsl`, shared test `shaders/hiz_test.wgsl`); shadows drawn, in each cascade, with only the objects whose shadow can fall on what the view reads there (`shadow_cull.rs`: volume of the pixels that read the cascade, stretched towards the light, tested by the GPU culling and the CPU lists; opaque faces without a fragment shader), and an opaque object drawn in one cascade only, the next cascade taking that part of its map from the finer one (`shadow_merge.rs`, `shaders/shadow_merge.wgsl`); streaming uploads with no copy on the main thread (`upload.rs`: mapped staging memory that the background tasks write into, GPU copies recorded in an encoder submitted at the head of the frame); textures compressed to BC7 when they support it without damage (`bc7.rs`: Intel ISPC encoder and quality check of each level; pages are in RGBA8 or in BC7 depending on the format of the chain, `TexelFormat`); nothing that grows or is freed all at once within a frame (`arena.rs`: geometry ranges allocated best-fit through an index of sizes, draw records kept in blocks and GPU buffer grown by a GPU copy; `textures.rs`: shrinking a texture by copying its levels to a smaller layer, grey substitute shared by the pending textures, emptied pages destroyed a few per frame by a separate thread); presentation mode of the swapchain (`present.rs`: a single rule at the creation of the surface and at every toggle of vertical sync, mode forced by `AURORA_PRESENT_MODE`); two halves, see "Threads and data ownership": `main_thread.rs` (the `Renderer` as seen by the scene: its stores, the closing of the frame), `writes.rs` (log of the frame's GPU writes), `packet.rs` (frame packet and result), `ui_frame.rs` (egui shapes handed to the render thread, which tessellates them like `egui::Context::tessellate`), `thread_cpu.rs` (CPU time of a thread, for the profile), `render_thread.rs` (render thread, rendezvous handoff of the packets), `renderer.rs` (`Backend`: surface, pipelines, targets, passes), `helpers.rs` (helper threads of `finish`) |
| `aurora-audio` | Audio output: mixer with the SL volume channels, music streams, world sounds |
| `aurora-voice` | SL voice over WebRTC (receiving and sending) |
| `aurora-media` | Host of the SLPlugin media plugins (CEF for the web, LibVLC for video), taken from Aurora's media plugins pack or from a folder chosen in the preferences |

Dependencies between crates (simplified):

```
aurora-viewer ──► aurora-net ──► aurora-msg, aurora-llsd
      │      └──► aurora-render ──► (wgpu)
      ├──► aurora-prim, aurora-assets
      ├──► aurora-audio, aurora-voice, aurora-media
```

## `aurora-viewer` in detail

Avatar name tag height is evaluated with each avatar's skeleton in
`scene/anim.rs`: shape and mesh joint offsets, plus the upper translation
envelope of each animation track (cached by `BoundAnim` when loaded).
The neutral upright height ignores animated rotations and sample-to-sample
bobbing. `Scene::name_tag_anchor` keeps it above the root with 0.2 m clearance;
both drawing (`ui/hud.rs`) and click depth (`App::pick_name_tag`) use that anchor.
This deliberately differs from Firestorm's posed-head tracking, at the
human's request (#261). An animation change may change the height envelope.

Account login: `login_accounts.rs` keeps the per-grid catalogue, each account's start location, remember preferences and last-place image reference without secrets, and merges changes under an OS file lock. `credentials.rs` stores password hashes and MFA authorizations in separate OS credential entries per grid/user; MFA expires after 30 days and is saved only when requested. `atomic_file.rs` publishes complete files through a unique temporary per write. `ui/login/carousel.rs` presents account cards with their own grid/start selectors, keyboard selection and the separate Add button. `ui/login/credentials_form.rs` shares the full first-account/Add form and the compact password/MFA prompt for existing profiles; registration happens after successful login. `ui/login/manager.rs` provides searchable portrait rows, saved account ordering and account-scoped credential removal, with confirmation keyed by identity. `ui/login/manager_drag.rs` tracks row gestures, insertion targets, a floating identity preview, Escape cancellation and edge scrolling; order changes happen once on release. Reordering moves identities in the latest lock-protected catalogue without replacing profile metadata or touching credentials. Login dialogs use `widgets::Modal::login_surface`: the existing GPU modal backdrop blurs the sky and lower account cards, with the login cards’ lighter tint inside the theme’s frame. Ordinary world modals retain their scene-glass markers; opaque skins skip the login blur. The floating drag preview shares `aurora_bg::frosted_pane` with login cards. `ui/login/derez.rs` prepares fragments on a background worker and batches them by GPU texture. `login_places.rs` resizes scene captures off the frame thread, publishes unique images atomically and loads bounded thumbnails; metadata updates only the matching grid/user/agent. Portraits and scene images (at most 512 pixels wide) live in each account's own folder. Place capture piggybacks on the existing scene capture at logout/quit, without interface overlays or a recurring readback.

`ui/login/layout.rs` caps central cards at three and large-window growth at 22%, reserving neighbour previews and the right information column. `ui/login/chrome.rs` shares the version/news panels, compact information dialog and community/version footer between the account and legacy login screens.

`ui/login/motion.rs` owns card/side dragging, circular browsing and a short velocity-projected snap. A click-and-drag response below the card children waits for egui's movement threshold, preserving simple click-to-connect and text-field drag selection. The carousel renders clipped neighbouring cards while moving; pointer deltas are consumed once across egui sizing passes. Above three profiles, browsing highlights the nearest central card and updates the selected identity only after settling; saved credentials and per-account metadata remain intact. Keyboard/button/wheel navigation centres the selected card immediately, using an odd number of full cards. Three or fewer profiles keep a fixed row and retain their positions when selected. Discrete wheel events across the whole browse rectangle are consumed once before vertical scrolling; hover animates the shared surface border and glow without changing geometry or cursors. Drag releases cannot initiate login, and child selectors/buttons retain pointer priority.

When accounts exceed the central capacity, neighbours remain visible at reduced opacity. `ui/login/fade.rs` reuses their tessellated meshes, clips triangles at the fade boundaries using the same UV-preserving geometry as derez, and lets the GPU interpolate transparency into the sky. Central cards keep their original shapes without extra tessellation. Motion projects release velocity over 0.14 seconds, coasts up to two additional card strides and settles more gradually.

The derez transition emits one login request after 0.1 seconds while centering the chosen card in 0.62 seconds and finishing fragments by 0.7 seconds. Its action-area snapshot anchors live server progress, preserving the Aurora background until authentication returns. Fragment preparation never delays the connection request. Login success/failure clears this presentation before region loading or a credential challenge; first/Add forms display the same progress helper and disable editing while pending.

`ui/scripts/indent.rs` handles the four-column stops for Tab and Backspace
and keeps the indentation on Enter in both editors, with egui's native cursor
and undo history. Pasting stays exact. `ui/scripts/experiences.rs` tracks the
association and the choices available in object scripts;
`aurora-net/src/scripts/experiences.rs` loads GetMetadata /
GetCreatorExperiences / GetExperienceInfo in the background. The chosen
association is part of the document and of the saved snapshot: saving no
longer needs to read the metadata again.

Typing `}` aligns its indentation with the matching `{`, outside strings,
comments and directives. `ui/scripts/colors.rs` shows swatches next to
numeric vectors between 0 and 1 inclusive, detected and cached by
`syntax.rs`. The shared colour picker keeps a window identifier of its own
for each document and shares the palette preferences with building. The
preview is local, Cancel restores the original bytes and OK forms a single
action in the native history. Typing in the source and saving wait for the
colour picker to close; an external change closes the session without
overwriting the new text.

`ui/scripts/editing.rs` groups the transformations on the selected lines and
the comment toggles, with a single Undo / Redo step and the direction of the
selection preserved. Keyboard commands that arrive after native typing or a
native cursor move wait for the cursor actually computed by egui.
`ui/scripts/folding.rs` tracks the pairs of braces outside comments /
strings, remaps the folds after edits and reduces to zero the height of the
hidden lines in the layout. The text and its indices stay complete; the copy,
save, compile and undo actions use no projection. Navigating / searching to a
hidden character reopens its block.

`ui/scripts/syntax.rs` colours the LSL source of both editors, also when
read-only. The catalogue of Firestorm's native names is embedded in
`assets/lsl/keywords.json`. The lexical analysis and the formats stay cached
per document as long as the source, the theme and the font do not change; the
source sent to the simulator keeps its bytes exactly when the preprocessor is
disabled.

`ui/scripts/view.rs` composes the menus, the toolbar, the line gutter and the
clickable diagnostics. `editing.rs` shares the native cursor and history
between the commands and typing; `dialogs.rs` carries search/replace, the
preferences and the keyword help.
`settings.rs` keeps the editor's colours and options in the common settings.
`reference.rs` exposes the signatures and descriptions of the Firestorm
catalogue embedded in `assets/lsl/reference.json`.
`files.rs` carries out the dialogs, the imports/exports and the watching of
an external editor in the background, with no shell and no silent overwrite
of a concurrent change. `world/script_debug.rs` filters the output of owned
scripts after the blocks and the bridge; `ui/scripts/debug.rs` displays it
all together or per object.

`ui/scripts/preprocessor/` prepares the save on a separate thread: lexing,
C macros and conditional expressions, includes of scripts from the
inventory, optional local includes, LSL transformations of extended lists
and `switch`, removal of unused functions/variables. The include requests
and their replies go through the editor's actions; no network or file wait
blocks a frame. The original source is wrapped with Firestorm's markers and
decoded on opening, while the simulator compiles the preprocessed result.
The two views use separate cursors: viewing the result replaces neither the
source nor its undo history.
`preprocessor/clock.rs` produces the date and time macros: local clock of
Windows, with a system call limited to the SYSTEMTIME structure; UTC for the
other platforms.

Scripts use a shared editor (`ui/scripts.rs`, actions in `app/scripts.rs`) for inventory items and the contents of a prim. Each document keeps its source, its saved version and its pending requests. `aurora-net/src/session/scripts.rs` loads the sources with `TransferRequest` and the inventory's permission context, including when the asset UUID is hidden: the simulator resolves the source from the item and its owner / object. A null asset UUID does not mean a new script and does not trigger the local template. `aurora-net/src/scripts.rs` saves and compiles through `UpdateScriptAgent` / `UpdateScriptTask`, with separate returns for the saved asset and the compile diagnostics. No transfer blocks the frame loop.

| Folder / file | Contents |
|---|---|
| `imports/`, `app/imports.rs` | Per-file import drafts removed after acknowledged uploads (discarded on a worker), local previews and unfinished drafts kept; preview, selection of the material maps, texture / animation / LOD / physics parameters; sequential upload queue with deduplicated material dependencies, mesh quote frozen before confirmation and processing of the files on the workers. `local.rs` watches the local textures, materials and meshes and prepares their GPU resources; meshes, textures and materials target the selection or a support prim through visual overlays kept in `world/objects.rs`, without changing the server data. The textures use the scene's staging pool; the render loop installs and frees their GPU slots explicitly. The local picker recognises the three types, including glTF/GLB documents according to their content. `persistence.rs` saves the references/settings and the applications per account/grid in the private profile; read back on a worker, atomic writes batched and flushed on close, missing sources kept. Connection, missing quote, unknown price and account-specific mesh permission denial have distinct statuses. `ui.rs` shows the same Name / Description / Destination rows above every import panel, including the mesh sections; `ui/destination.rs` filters the allowed folders, with a cache and a virtualised list; `ui/material_panel.rs` keeps the 3D canvas and gesture hints stable while loading, beside four distinct PBR cards in a 2 × 2 grid without scrolling, with left-aligned titles and names, right-aligned actions and common parameter rows; `ui/mesh_panel.rs` places the preview beside LOD cards selectable anywhere in their frame, sources and criteria on the left and results / retained percentage on the right, followed by collision and upload options; `ui/zoom.rs` enlarges the 3D view with its camera or a square texture preview prepared on a worker at native resolution (2,048 pixels per side maximum, one large GPU cache at a time); `permissions.rs` stores per-type next-owner defaults with migration from the legacy shared preferences, frozen into each confirmed upload step; `ui/permission_panel.rs` exposes only Modify / Copy / Transfer for the five importable types in the main Permissions tab; `animation.rs` keeps an independent clock; `preview.rs` also prepares the imported hand poses, expressions and joint positions |
| `aurora-assets/src/import_*` | Pure validation and conversion: dimensions / alpha / JPEG2000, WAV to Vorbis, BVH to animation, RGB / ORM PBR maps, shared glTF / GLB resources, Collada / glTF geometry, LOD through meshoptimizer, spatial transfer of the weights, convex hulls and serialisation of the mesh into compressed LLSD sections. `import_mesh/vhacd.rs` and `native/vhacd/` wrap V-HACD 4.1: bounded allocations, contained exceptions, atomic cancellation / progress; each model runs on a Rust worker |
| `aurora-net/src/inventory/{upload,mesh_upload}.rs`, `session/asset_imports.rs` | Two-step import protocol, expected cost, read-back of the created item; materials created then saved through their dedicated capability. The mesh transfer URLs stay private to the session, tied to a local token consumed only once and expiring after five minutes |
| `aurora-render/src/preview.rs`, `shaders/preview.wgsl` | 3D previews in the windows: PBR rendering and skinning on the GPU thread, off-screen targets linked directly to egui, deduplication of the shared maps, no GPU readback and no wait on the GPU in the interface loop. Shared orbit, screen-space pan, zoom and camera reset; material previews use Sphere / Cube / Aurora 3D and four worker-prepared thumbnails, with a self-contained bundled GLB |
| `build/replace.rs`, `build/replace/`, `build/ui/replace.rs` | Guided replacement of a linked part: locked target, selection of the replacement, options and progress; steps confirmed by the server, copy of the contents through the resident's inventory, script states preserved, recovery of the initial configuration and offline simulator |
| `ui/inventory/document.rs`, `aurora-net/src/inventory/notecard.rs` | Editing cycle of the documents: draft and save correlated with the window, closing after confirmation, refusal that keeps the text; import of v1 / v2 notecards, v2 export with a bounded UTF-8 length and the permissions of the attachments still referenced preserved |
| `ui/inventory/embedded.rs` | Items embedded in notecards: icon and name in the text, preview / opening and copy to the inventory, drop at the pointer position; one character per item to preserve selection, deletion and undo |
| `aurora-net/src/session/inventory_preview.rs` | Reading of the notecards and scripts protected by the simulator's inventory transfer (identifiers of the agent, the session, the owner, the object, the item and the asset); bounded reassembly of out-of-order packets, correlation with the simulator and expiry |
| `app/task_notecards.rs`, `aurora-net/src/session/task_notecard.rs` | Notecards in the contents of objects: double-click / Open, shared editor with a state specific to the object and to the item; rights of the object and of the notecard, asset transfer authorised by the inventory, save through UpdateNotecardTaskInventory, read-back and draft kept on refusal |
| `main.rs`, `app.rs`, `app/` | Startup, event loop, one frame of the viewer, interface actions; chat bar commands (`app/chat_commands.rs`), right-click menus (`app/context_menu.rs`: opening, rights on the object, chosen actions), clicks on the world without waiting for the GPU (`app/click_pick.rs`: the depth under the click is requested from the renderer, the click is handled when the reply arrives with the cursor and the keys it had, and what follows it — release, other buttons, mouse movement that moves the camera — waits behind it in order; check `AURORA_CLICK_PICK`, demo clicks `AURORA_DEMO_CLICKS`), HUD move gesture with a configurable key (ALT by default) and left click, and offline scenario (`app/hud_drag.rs`) |
| `cli.rs` | Command-line arguments (`--title`, `--fps-limit`, `--language`, `--version`); title with the common version and the build profile |
| `cmdline/` | Chat bar as a command line (FSCmdLine): parsing of the commands, calculator (LLCalc), dice; executed by `app/chat_commands.rs` |
| `agent.rs` | Our avatar (AgentUpdate, extrapolation from the server velocity / acceleration, smoothing like LLDrawable) |
| `world/motion.rs` | The other avatars and the objects between two reports of their simulator: extrapolation along the clock of their region (`ObjectStore::motion_time`, advanced by `World::advance_motion` at the region's time dilation, stopped with a silent circuit), and the fading of the gap a report opens with where the object was drawn (`Correction`; while it fades the object counts as moving for the scene, `ObjectStore::expire_corrections` ends it) |
| `ao/`, `app/ao.rs`, `ui/ao.rs` | Built-in Animation Overrider: catalogue of the `#Firestorm/#AO` sets, inventory links and ZHAO import (`inventory.rs`), states / cycles / groups / tracks and remapping of the pose of our avatar only (`runtime.rs`), synthetic assets and transitions (`demo.rs`); AIS operations and asynchronous replies in `app/ao.rs`, management window opened by the AO button of the bottom bar. The raw signals of the simulator are still used for movement and transitions; the overrides are announced through AgentAnimation. Activation and last set saved per account UUID; pause and standing poses specific to the session. |
| `camera/` | The camera like LLAgentCamera: view behind the avatar, Alt camera on a point or an object (`focus.rs`), seat camera, scripted camera (`follow.rs`: LLFollowCam parameters, per-object activation stack held by World and avatar-relative simulation; `world/mod.rs` deactivates seat/attachment linkset sources before parent changes, retaining parameters and unrelated sources), mouselook, transitions, smoothing, pull-back in flight, collision sent by the simulator; Firestorm settings (`settings.rs`), demo scenarios (`demo.rs`) |
| `world/` | World state received from the network: objects, regions, terrain, objects of the out-of-range regions set aside until these close or come back into range (`parked.rs`), environment (EEP: cycles and settings assets in `eep.rs`, choice local > parcel > region > default day, local environment and fades in `eep_env.rs`, lists of the environment picker and loading of the folders in `env_select.rs`), selected parcel of "About Land", its rights and its refresh while the window is open (`land.rs`), social, avatar profiles, inventory, groups (active, pinned), contact sets and nicknames (`contact_sets.rs`), blocks, messages of Firestorm's LSL bridge (hidden), body orientation, gaze (LookAt), map, place profiles of "Places" and of the separate windows (`place_details.rs`: region by its name, landmark, RemoteParcelRequest, ParcelInfoRequest), landmark assets and handles of their regions (`landmarks.rs`), dated and saved teleport history (`tphistory.rs`)… |
| `scene/` | What is sent to the GPU: geometry and LOD, textures (streaming), avatars (skeleton, animations, silhouette), particles, water, reflection probes, world sounds, impostors, environment settings assets (`settings.rs`: sky, water, day cycle) |
| `ui/` | egui interface: bars, windows, options, chat, contacts (friends, groups, contact sets: `contacts.rs`), avatar profiles, "Places" (`places.rs`: favourites, landmarks, teleport history) and place profiles (`place_details.rs`, also as separate windows), "About Land" (`land/`, one file per tab, dialogs), group profile (`group_profile.rs`, first version: the group card of "Search" in its own window), resident picker (`avatar_picker.rs`), web page in a window (`web_view.rs`), texture picker, inventory, environment picker and "Personal lighting" (`environment.rs`), maps, debug overlays; sounds caused by the widgets (`sound_cues.rs`: clicks, refused keys, windows); right-click menus: shared look (`menu.rs`: Tabler icon, submenus, checked boxes) and entries like Firestorm (`context.rs`: world, avatar names, groups) |
| `build/` | Building tools like LLFloaterTools: selection and manipulators (`manip.rs`), selection regressions for linked parts and whole objects (`selection_tests.rs`), cycling through the selection under the pointer with CTRL + wheel (`cycle.rs`: units per linkset or prims of a single object, high-precision wheel), direct moving of HUDs (`hud_drag.rs`: root of the linkset, screen / attachment point conversion, local echo and position MultipleObjectUpdate on release), Move (`grab.rs`) and Align (`align.rs`) tools, terrain (`land.rs`), edits of the selection (`edits.rs`), materials and media (`materials.rs`), object contents (`contents.rs`), impact and weights (`costs.rs`), demo simulator (`demo_sim.rs`); the window and its tabs in `build/ui/` |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Rules of the click actions 0–9, inheritance, permissions, cursors drawn from the Tabler icons (Firestorm's rules), buy / pay windows and contents list; transaction after confirmation |
| `app/object_actions.rs` | Triggering of the actions, held touch, physical move, parcel playback, opening of media and camera framing |
| `ui/buy_currency.rs`, `app/buy_currency.rs`, `aurora-net/src/currency.rs`, `aurora-net/src/session/currency.rs` | Buying L$, opened from the balance of the top bar: window and estimate / confirmation / purchase cycle (LLFloaterBuyCurrency, LLCurrencyUIManager), choice of the grid's help page, end notification and simulated service of the demo; XML-RPC calls `getCurrencyQuote` / `buyCurrency` (sent only once, parameters never logged) |
| `scene/picking.rs` | World rays against triangles shared with displayed prim / sculpt / mesh geometry; exact hit prim rather than overlapping bounds, IGNORE traversed outside build mode, Touch surface data. Right click and edit selection include transparent surfaces without an action, testing candidates in parallel while keeping displayed-depth occlusion; fully invisible objects retain their CPU geometry in the pick index. Blend presses and hover resolve the material mode per intersected face, independent of texel opacity. |
| `scene/drawn.rs` | Click target from the GPU-posed faces under the pixel (aurora-render/src/drawn_pick.rs): animesh, rigged attachments and worn meshes reached where displayed by left click, right click and build tools. Isolated gesture × object-status rule (reaches), posed Touch surface data (face, position, ST / UV, normal and binormal). Blend stops presses without an action and at zero texel alpha; Mask retains its alpha test in GPU picking. The posed point bounds media and prevents fallbacks to a background action or avatar. Deliberate difference from Firestorm, which reaches rigged meshes only by right click and the Move tool. |
| `scene/list_index.rs` | What the draw lists look for in each object slot every frame (avatar, transparent or glowing faces, drawn object, worn HUD): one byte per slot, written where the GPU's object table is written (after the sync of an object, when its faces are reclassified for alpha, when its slot is freed), so as not to walk the `ObjGpu` of every slot ever used; `AURORA_LIST_INDEX=0|check` |
| `scene/hover.rs` | Cached object and surface point under the cursor: compact sphere index maintained by sync, candidates near the ray or displayed depth, reused while the question and nearby scene stay unchanged, refreshed at least every 100 ms; never a click cache. Foreground Blend triangles block hover without an action and at zero alpha, retaining large-face hierarchies; Mask holes remain pass-through. The cached point bounds media hover. Always-tested objects (avatars and rigged meshes in crowds) have classes computed after arrival / change to exclude irrelevant steps. AURORA_HOVER_CHECK compares the full reference search; AURORA_HOVER_LEGACY compares the index / hierarchy strategy; hover summary provides profiling. |
| `scene/pick_accel.rs` | Triangle bounding-volume hierarchy for a picking face: the same ray distance as exhaustive triangle tests, built by a background job when a large mesh / sculpt face is first queried. Hover and the shared Blend query use ready hierarchies; other click surface tests remain exhaustive. |
| `media/` | Media of prims and parcels (objects with media kept up to date by the change stream of the `ObjectStore`, with no pass over all the objects), OpenID cookie of the grid's web pages (`openid.rs`), private browser profile deleted after use for the Web tab of "Search" (`private_profile.rs`: its own folder under `cache\web-private`, usage lock, deletion when the browser stops, on exit and at the next startup), download of Aurora's media plugins pack when no plugins are found, with retries (`pack.rs`) |
| `demo_chatlog.rs` | `AURORA_DEMO_CHATLOG` scenario: history files of the demo account (in its own folder), conversation with history, search, History window on a long file, IMs received offline, long conversation to measure the view |
| `demo.rs`, `demo_land.rs`, `demo_place.rs`, `demo_eep.rs`, `demo_env.rs`, `demo_stream.rs`, `demo_asset_server.rs`, `demo_glyphs.rs`, `demo_tags.rs` | Demo mode: a local scene that simulates a server (and its replies to "About Land", to the place profiles, landmarks and history of "Places" and to ExtEnvironment, a library of environments for the picker); `demo_stream.rs`: objects and textures that arrive in waves (test of the streaming hitches, AURORA_DEMO_STREAM), and `demo_asset_server.rs`, the local HTTP server its `net=` option downloads the textures from, at the round trip of a distant grid; `demo_glyphs.rs`: residents whose title, name and message use the extended characters of display names (AURORA_DEMO_GLYPHS) |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Saved settings, shortcuts, Windows keyboard layout and default movement keys, palette |
| `ui_sound.rs` | Catalogue of the interface sounds (Firestorm's UISnd*), per-sound settings |
| `ui/appearance.rs`, `ui/appearance/items.rs`, `ui/appearance/gallery.rs`, `world/appearance.rs` | Appearance window, gallery / outfits / worn, editing and Save as dialog; menus of the items and of the gallery, save / delete confirmation, image choice, renaming; attachment points / HUD, profile and original; rules for changing the COF, saving through links, protection of the body parts, offline scenario |
| `aurora-net/src/outfits.rs`, `aurora-net/src/outfits/categories.rs` | Writing of the outfit links, favourites, names and images through AIS InventoryAPIv3, move to the trash; confirmation and read-back of the folders / items |
| `slurl.rs`, `link_trust.rs` | Links in texts: SLURL (navigation bar, "Region (x,y,z)" labels of the place links); trust of web links (trusted site, unknown, dangerous: shorteners, fake official names, IP addresses…), judged locally on the URL |
| `ui/search.rs`, `ui/search/details.rs`, `world/search.rs`, `aurora-net/src/search.rs`, `aurora-net/src/session_search.rs`, `demo_search.rs` | "Search" (Firestorm's legacy search): directory queries and their flags, reading of the replies and of the event / group details (`aurora-net/src/search.rs`, pure; sending and receiving in `session_search.rs`); one result list per tab with the pending query, pages of 100, statuses, sorting and selection (`world/search.rs`); window with seven tabs, forms, sortable list and web page in the built-in browser (`ui/search.rs`), detail cards and their actions (`ui/search/details.rs`); offline directory of the demo (`demo_search.rs`, AURORA_DEMO_SEARCH) |
| `ui/find_shortcut.rs` | Ctrl+F requested by three features: the single rule (`owner`) that gives it to the window one of whose fields has the keyboard (script editor, conversation), otherwise to the conversation under the pointer, otherwise to the viewer's shortcut ("Search"); the windows with a search bar declare themselves to it every frame (`take`), the application queries it before running its shortcut (`window_takes_it`) |
| `ui/widgets.rs`, `ui/widgets/resize.rs`, `ui/widgets/demo.rs` | Shared widgets and floating windows; collapsing by double-clicking the title or by a button, expanded size kept and collapsed bar movable; offline gestures on the real title; resize handles bounded before the move, opposite edge fixed, limits of the content and of the visible area; position and size kept by egui; names longer than their space cut with "…" with the whole name on hover (`elided_label`, `paint_elided`) for the lists of residents, groups and conversations |
| `ui/layout.rs`, `ui/layout/gesture.rs` | Interface layout: egui's memory (positions, sizes, collapsed windows) written to `ui_layout.ron` on exit and read back at startup; work area left by the bars (`work_area`) and common gap `GAP` of the default layout and of snapping (none: the windows touch); `Slot`, through which `widgets::Floater` and the minimap (`widgets::bare_window`) go, records where each window stood and in which screen, and makes it follow its nearest edges when the viewer window changes size, during the session or from one session to the next (`follow_edges`; a group of windows stuck together follows as a block). `gesture.rs`: the gestures of the frame (`layout::frame`, `layout::resizing`): window dragged by its title together with those stuck to it, block kept within the screen, snapping proposed then applied on release (under the pointer for a resize), sticking links created, saved with the layout and removed when they no longer hold (window closed, collapsed, moved apart, Alt) |
| `ui/theme_guard.rs` | Test only: reads the interface sources (`ui/`, `build/ui/`, `imports/ui*`, `crash/window.rs`) and fails if a window directly uses an egui widget replaced by a shared widget of `ui/widgets.rs`, or itself writes a radius or a colour instead of those of the theme (AGENTS.md, section 7); list of the rules, of their replacement and of the exceptions (files of `ALLOWED`, lines marked `// theme-guard:`), themselves checked |
| `ui/snap.rs`, `ui/snap/overlay.rs` | Window snapping, geometry only: which moving edges stick to what (edge facing another window, same-side edge of a neighbour, work area; 10 px margin, edges that touch (`layout::GAP`, no gap), size limits of a resize, holding of the displayed snap), guide to draw, side-by-side windows (`touching`), groups (`group`, `block_limits`). `overlay.rs`: the guide and the "Alt" pill drawn above the windows |
| `ui/numeric_drag.rs` | Continuous dragging of the numeric fields, including those of the sliders: virtual pointer fed by the relative movement, Windows cursor held near the field; release, loss of focus and disappearance of the widget free the pointer. Keyboard entry and slider bars unchanged |
| `ui/hud.rs`, `ui/nameplate.rs`, `ui/nameplate/mesh.rs`, `ui/nameplate/paint.rs`, `ui/nameplate/tests.rs` | Name tags above avatars and floating texts. `hud.rs` chooses the tagged avatars, their anchor point and what is known about them (names, status, voice, complexity), and draws the classic name tag. `nameplate.rs` is the name plate (default style, `Settings::classic_name_tags` for the old one): pure functions for the level according to the distance with its hysteresis (`level_for`), the lines, badges and status dot of each level, the measurements taken from the mock-up (`Metrics`) and the layout (`layout`), the voice envelope and its halo, and `Plates`, what each plate keeps from one frame to the next (level and transition, envelope, photos requested at most four per second, geometry). `mesh.rs` builds the geometry as simple meshes — pill, border, shadow and halo as rings around an outline, antialiased discs and rings — instead of egui's rounded shapes, tessellated again every frame; `paint.rs` draws a plate, keeps its geometry as long as its state does not change and redraws it without a copy when it has not moved |
| `ui/fonts.rs`, `ui/fonts/fallback.rs` | Interface fonts: main font, embedded or chosen, those of egui, then the system fallback fonts for the characters the others do not have (display names, titles, chat: Georgian, CJK, Arabic, mathematical letters…). `fallback.rs`: list per script, first installed font that draws the probe character, files memory-mapped and checked on the `aurora-fonts` thread, alignment of their baseline with the main font; installed on the next frame if the search was not finished. `AURORA_FONT_FALLBACK=0` removes them |
| `ui/exit.rs` | Translated exit modal: quit, log out to the login screen, cancel; background blurred on the GPU and interactions blocked; non-blocking wait for the end of the session, with Firestorm's six-second fallback timeout |
| `updater.rs`, `ui/update.rs` | Asynchronous controller and update window: notes, progress, restart and errors; manual check in Help / login, ignored tag and Ask / Automatic / Required only choice. Isolated synthetic demo through `AURORA_DEMO_UPDATE`. |
| `ui/news.rs`, `ui/news/markdown.rs`, `links.rs` | News of the login screen: Markdown files `news/<language>.md` of the repository, one per interface language (fallback to the base language, then English). At startup, the last download kept in the cache (`news-<language>.json`) or, failing that, the files compiled into the viewer; then the file from `main`, downloaded in the background (bounded size, 20 news items, texts cut, `https://` links only), replaces them. Reloaded when the interface language changes; a failure leaves the panel as it is. No download in demo nor in the test installation of updates; `AURORA_NEWS_URL` reads a local server or another branch, without cache. `markdown.rs`: tolerant reading of the format (`## date — title`, text, link), HTML comments ignored |
| `ui/inventory.rs`, `ui/inventory/view.rs`, `ui/inventory/controls.rs`, `ui/inventory/context.rs`, `ui/inventory/clipboard.rs`, `ui/inventory/delete.rs`, `ui/inventory/dialogs.rs`, `ui/inventory/properties.rs`, `ui/inventory/thumbnail.rs`, `app/inventory.rs`, `app/inventory_thumbnail.rs` | Inventory: pure sorting / filters and per-generation view cache in `view.rs` (index, sorting and filtering in parallel with rayon), persistent commands and preferences in `controls.rs`, filtered tree including the parent folders, expansion states per tab, results drawn according to the visible area without a limit of 500 and status area of fixed height; scrolling on both axes, width of the names measured in the visible area then kept for the virtual results, commands adapted to small widths; menus per type, multiple selection with grouped add / detach, renaming in the row after creation or on request, folder windows limited to their descendants with a filter of their own, clipboard shared between windows and native Ctrl+C / Ctrl+X / Ctrl+V shortcuts according to the focus of the list in `clipboard.rs`, Delete and bottom-of-window trash through the shared deletion in `delete.rs`, properties and confirmations, previews / editing of notecards and scripts, reading of the animation statistics; image editor with six actions, tree picker and screenshot of the scene without the interface; decoding / resizing / encoding off the render thread and execution of the actions after the interface is drawn |
| `world/inventory/actions.rs`, `world/inventory/wearable.rs`, `world/inventory/demo.rs`, `world/inventory/preview.rs`, `world/inventory/tests.rs` | Permission rules, moves / copies / links and offers, preparation of the Marketplace listings, default clothing assets and synthetic scenario; v2 inventory cache with full permissions |
| `aurora-net/src/inventory/operations.rs`, `aurora-net/src/inventory/thumbnail.rs`, `aurora-net/src/session/inventory_upload.rs` | AIS mutations with read-back, remapping of the UUIDs assigned by the server, UDP creations / copies and acknowledgements with expiry, saving of the documents through capabilities; AssetUpload / Xfer upload of clothing before the item is created; free thumbnails through InventoryThumbnailUpload, JPEG2000 POST then AIS and read-back |
| `aurora-assets/src/j2k/encode.rs` | Memory-bounded OpenJPEG encoder, square RGB / RGBA thumbnails of 64 to 256 pixels and alpha preserved |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs, disk cache, remembered password (OS vault) |
| `world/chat_log.rs`, `world/chat_log/`, `world/transcripts.rs` | Conversation history on disk, like Firestorm (LLLogChat, LLIMModel::logToFile). `chat_log/format.rs`: text of a line (`[YYYY/MM/DD HH:MM]  Name: text`, SLT time, following lines preceded by a space, `:` of a name written `%3A`) and its reading back; `names.rs`: file of each conversation (`chat`, legacy name of the resident, `<group> (group)`, dated conference), forbidden characters, reserved names of Windows, date in the names; `store.rs`: folder of an account on a grid (`accounts/<uuid>/chat_logs/<grid>`), `aurora-chatlog` thread that appends the lines (file opened, written and closed again for each batch, each task carrying its full path) and reads back the end of a file (LOG_RECALL_SIZE); `transcript.rs`: whole file for the History window, in pages of 100 messages, without keeping it in memory (index of the pages, page-by-page search); `chat_log.rs`: facade opened at login and closed at logout, settings. `transcripts.rs` links the conversations to the files: file name of a session, lines waiting for a name, history requested before the first write, history lines (`Origin::Log`, `Origin::ServerLog`) placed before the new ones, merge with the history of the group chat server, stable numbering of the lines for the view (`LineClock`) |
| `ui/chat.rs`, `ui/chat/view.rs`, `ui/chat/find.rs`, `ui/chat/history.rs` | Conversations window. `view.rs`: only the visible lines are laid out (heights measured and kept per width, off-screen lines measured without being drawn within a per-frame budget, scroll offset corrected when a line changes height above); `find.rs`: toolbar filter and Ctrl+F search bar specific to each conversation; `history.rs`: History window (pages, search in the whole file, folder) |
| `aurora-net/src/offline_im.rs`, `aurora-net/src/session_offline.rs` | IMs kept by the grid during the absence: decoding of the reply to `ReadOfflineMsgs`, request made once after login (capability, otherwise `RetrieveInstantMessages`), replies through capability to the friendship offers and group invitations received offline |
| `bug_report/`, `ui/bug_report.rs`, `ui/bug_report/tests.rs`, `app/bug_report.rs` | Bug report: explicit snapshot of the authorised data, form with Description / Picture / Data tabs, localised values that can be unchecked, export with English titles; public diagnostics with a closed vocabulary and images re-encoded off the main thread. Local export with separate public / private folders; prefilled GitHub link with clipboard fallback, or issue creation after the Device Flow of a GitHub App. `uploads.rs` declares the hashes and sends the public image and the optional private ZIP to the PHP / Cellar API, then adds the links; partial failure resumed on the same issue. The GitHub authorisation stays only in memory in `github.rs` and is not passed to the Aurora API. No SL login secret, chat or IM enters the public model. |
| `screen_rate.rs` | Refresh rate of the screen under the window, read again once per second (window moved, screen mode changed): cap of the login screen and frames already paced by vertical sync |
| `window_hold.rs` | Frames while Windows holds the window (mouse button pressed on the title bar, a border or a title bar button, move, resize, system menu): Windows follows those in a message loop of its own, in which winit's loop does not turn and no redraw is asked. The window procedure is hooked to know when a hold begins and ends, and a helper thread then asks for an immediate repaint, which winit delivers as a `RedrawRequested` on the main thread. During a hold the frame limiter does not wait on the main thread (`paces`): the helper waits between two frames, and spaces them out while the cursor moves so that the window keeps following it |
| `frame_profile.rs` | Frame profile (AURORA_PROFILE): time of each step of the frame, summary line per second in the log (average and maximum of each step, number of slow frames; detail of the streaming on the main thread, `s_*`, of the frame parameters, `p_*`, and of the interface per group of windows, `u_*`; time spent off the CPU per step, `off_cpu:`; meeting of the main thread and the render thread, `thread` segment) |
| `net_profile.rs` | Network profile: what the link and the session did over each second, from the counters of `aurora-net` (`NetStats`: packets, resends, sequence gaps, longest pause of the session loop, delay between a key and its AgentUpdate), the wait of the events before the main thread applies them and the corrections of our avatar (`agent.rs`). Read by the Performance window; one `net summary` line per second in the log with AURORA_PROFILE |
| `scene/lights.rs` | Local lights of the frame: the prim lights closest to the camera (range × 1.5, falloff, colour) and, for those that have a texture, their projector — cone computed from the position, the rotation and the size of the prim (`aurora-render` `Projector`), texture reserved as long as the light stays in the list; choice of the two projectors that cast shadows (the most present on screen) and fade of their shadow maps (`SpotShadowSlots`) |
| `scene/animesh.rs` | Standalone skeletons of the animated objects, animations of the linkset, bounds of the poses for culling and shadows, demo scenario |
| `scene/morph.rs` | Morphs of the system avatar body: the visual parameters that move the vertices of the base meshes (sex, body fat, face, system hair, eyelashes, skirt). A body part is built by a job for a set of weights and shared by the avatars that have them (`GeomKey::AvatarMorph`); hidden parts are not built |
| `scene/skin_binds.rs` | Skin bindings sent to the GPU (inverse matrix and palette bone of each bone of a rigged mesh, default skeleton first): one range per mesh, returned when its last geometry is freed and reused by the following meshes; only the modified ranges are sent |
| `scene/sync_sets.rs` | Objects that the scene sync visits every frame, kept up to date by events (modified objects noted by `ObjectStore`, set of the objects that move by themselves and of what follows them, pending geometries, level-of-detail checks: still objects due when the camera has moved enough, objects placed by the GPU in turn (every 24 frames if the camera moves, every 0.4 s otherwise), every object every 10 s; worn HUDs listed on their changes and not with their wearer) instead of a walk over all the objects; the old LOD slice of all the objects stays behind `AURORA_SYNC_SLICE`; queue of the full syncs that did not fit in the frame's time budget, served from the nearest to the farthest (`Backlog`, `FullSyncBudget`); queue of the new objects, which wait for their first sync in arrival order without going through the frame's list (`Arrivals`, marked by the `ObjectStore` until that first sync) |
| `scene/lod_watch.rs` | When to review the level of detail of a still object: each check leaves it the distance the camera can still travel before `lod_for` can change level (`lod_slack`, rounded down), and the object becomes due again when the camera has travelled it (queue sorted by deadline); a settings change (LOD factor, field of view) makes everything due. The check itself remains that of the sync: same levels at the same distances |
| `scene/sync_check.rs` | `AURORA_SYNC_CHECK` diagnostic: per second, the objects the sync visits per frame, according to what listed them and to their nature, and what came out of it (checked, re-placed, full sync) |
| `scene/still_check.rs` | `AURORA_STILL_CHECK` diagnostic: per second, what changed while the frame should be still (camera, our avatar, rebuilt faces and their reason, placed objects re-placed) |
| `scene/sync_plan.rs` | Parallel placement (`frame_par.rs`) of the frame's objects, level by level of the parent chains: transform, LOD, bounds; update of the matrix alone or full sync |
| `frame_par.rs` | Parallel loops of the frame in which the calling thread takes part (placement of the sync, draw lists, poses, flexible prims): it starts the loop at once, a few workers of the rayon pool join it as they wake up, and those that arrive too late are not waited for. The number of helpers follows the amount of work (`Grain`); a short loop stays on the calling thread |
| `scene/anchors.rs` | What avatars wear, placed by the GPU: each bone palette ends with anchors, rigid world matrices written every frame — the first places the skeleton (skeleton space to world), the following ones the attachment points of the body as they are posed. The record of a worn prim contains its placement relative to the anchor of its point (`flags::ANCHORED`), that of a rigged mesh (worn mesh, bento, animesh worn or on the ground) and of an avatar's body reads the anchor of its skeleton: the vertex shaders compose. The GPU culling likewise composes the sphere of the worn prims (`CullObject::anchor`) and computes every frame that of the rigged meshes from the per-bone boxes of their geometry (`JointBoxes`, `object_flags::POSED`). The sync writes the anchors in use, sends them with the posed palettes, keeps the centre of the anchored objects up to date for the CPU (sorting of the transparent faces, hover, lights, impostors; the lists the CPU still builds test a rigged mesh with the extent of its skeleton) and no longer visits these objects as long as only their wearer moves. Left to the sync: HUDs, flexible prims, objects that move by themselves and what is linked to them |
| `scene/texture_cache.rs` | Disk cache of the transcoded textures (`cache/tex/<id>.bc7`): BC7 mip chain produced by a decoding task, alpha class, picking mask and header of the JPEG2000 image; read back at the wanted level without decoding when returning to a region, written by the `aurora-texcache` thread; a file with no levels records that a texture must stay in RGBA8 |
| `scene/texture_load.rs` | Measurement of the texture loads: time of the decoding tasks per step (codec, conversion, RGBA, mips, staging memory) and `texture load:` line of the log at the end of each load (duration, decodes per level, bytes downloaded, disk cache) |
| `scene/texture_budget.rs` | Texture memory budget: current target (the setting, less when the rest of what the viewer holds on the card leaves less room under the budget that the driver grants it, `aurora-render/src/vram.rs`), and pressure level when the pages exceed it. Under pressure, `scene/textures.rs` shrinks the textures loaded more finely than their on-screen size, off-screen ones first, by copying their levels on the GPU (`TextureTable::shrink`), without decoding them again; the pages they leave are compacted without waiting. Override: `AURORA_TEXTURE_BUDGET_MB` |

The voice indicators of the classic name tags are drawn by
`ui/voice_tag.rs` (the name plates have their own outline and halo,
`ui/nameplate.rs`): outline smoothed according to the speech level, colours
calibrated on the WebRTC scale (red from 0.58, the local microphone peaking
around 0.63), struck-through Tabler microphone (`microphone-off`) for the
avatars outside voice. `voice.rs` guarantees the entry of our avatar from the
local microphone, even without an echo of its presence from the voice server.

## Life of a frame

`build/task_inventory.rs` centralises the permissions and the destinations of
the transfer of an object's contents to the personal inventory;
`ui/inventory/task_drop.rs` provides the common source of the two contents
windows, the drop targets and the confirmation of removals.
`app/inventory.rs` checks the permissions again before the
`MoveTaskInventory` message, with two persistent warning preferences
(ordinary object or object containing scripts).

Face selection keeps a mask of TextureEntry indices per prim in
`scene/mod.rs`. The world and HUD draw lists carry a highlight pass for the
visible faces, coloured by the palette, without changing the materials. Each
draw keeps its true face index, even when transparent faces are omitted.
Selecting an opaque face does not rebuild its records; a selected, fully
transparent face temporarily receives a record meant for the highlight
only.

`aurora-render/src/selection.rs` and `shaders/selection.wgsl` compute an
outer halo from the coverage of the selected triangles, with the depth of
the world or of the HUD and the usual posed geometry. Two separable passes on
the GPU follow the silhouettes, the holes and the disconnected pieces,
including a closed face with no border edge. The halo is composited after
post-processing, without touching the inside of the faces; a brief tint on
click fades out in 0.7 s. "Show selection again" restarts this tint without
changing the objects. `scene/selection_demo.rs` provides the offline mesh
with separate curved islands of the `faces-complex` scenario, next to a
closed torus primitive.

`ui/texture_picker/tree.rs` shares the search cache and the virtual tree of
`ui/inventory/view.rs`. The folders load in bounded batches; the expanded
states of search and of navigation stay separate. `ui/texture_picker.rs`
presents the visible rows, the long names truncated with a tooltip, a
bounded preview, an adjustable split and the commands at the bottom of the
window. The inventory column is reduced by a third by default. The
collapsible "Local files" folder precedes the inventory for the diffuse and
PBR material choices; it shares the drawing of the rows and filters
textures / materials separately. `app/imports.rs` prepares the local images
and materials as soon as they are loaded on the workers, so that they can be
selected before their first application.
Its intentional choices are distinct from the confirmation:
`build/ui/texture_preview.rs` applies the immediate preview to the captured
selection, keeps the original textures and materials of each face, confirms
without resending with "OK" or restores with "Cancel". The other users of
the picker keep their usual confirmation; non-copyable inventory textures
produce no preview, as in Firestorm.

The `ui/inventory/move_to.rs` picker offers the destinations of
"Move to…" for the original selection, with a filter by name and
paths. The list is computed in parallel, guarded by generation and drawn
only for the visible rows; the move reuses the existing inventory
mutations, with a check of the rights on validation.

Parent changes go through `aurora-net/src/session/inventory_moves.rs`
(reliable UDP MoveInventoryItem / MoveInventoryFolder, like Firestorm).
Moves alone publish `InventoryMoved` as soon as they are sent: the tree
changes immediately, without waiting for an HTTP read-back. The status shows
"Move requested"; the server updates reconcile the model.
Mutations that mix moves and metadata keep their AIS check.

`inventory/operations.rs` remaps the UUIDs of the folders confirmed by AIS
and returns their metadata, without reading back folders that are still
empty. The model can thus receive their items immediately.
`session/inventory_copies.rs` sends the reliable copies and tracks their
confirmations; the empty name is encoded with a zero length, like
`LLTemplateMessageBuilder::addString(std::string)`.
The reception of the items completes the copy, without imposing a second
HTTP snapshot.
The creations without copy of the callers of `mutate` / `mutate_with_moves`,
in particular the replacement of prims, keep their confirmation by read-back
before continuing their work.
The folders published directly in the inventory use the normalised name
sent to the server, with the same rules of characters and length.

`session/inventory_updates.rs` receives `UpdateCreateInventoryItem` and
`BulkUpdateInventory` directly over HTTP, like Firestorm's `LLEventPollImpl`,
and the LLSD variants of `LargeGenericMessage` / `GenericMessage`, in
addition to the UDP transport. `InventoryUpdated` applies items and folders
as soon as they arrive. `world/inventory.rs` advances the versions of the
loaded parents, ignores older replies and avoids counting repeated records
again.

`ui/inventory/progress.rs` distinguishes the submission of a mutation from
the tracking of the copies in the background: several copies can be tracked
without disabling the other commands. `InventoryProgress` passes on steps
and counters; the status line, shared between windows, shows the activity
and the elapsed time, then the end confirmation. Errors keep the possibility
of reloading the folders concerned, without replaying the copies or the
moves.

### Window surfaces

`world/radar.rs` holds the radar of the People window (sweep of the avatars once per second, first
appearance, crossings reported, regions left behind by a teleport), `ui/people/radar.rs`
its table, its settings and its options menu; `ui/people.rs` the Friends, Groups and Blocked tabs.
The "Invisible" status (right to see us online withdrawn from each friend, then given back to those who had it)
is in `world/status.rs`.
`ui/widgets/keycap.rs` draws the keys of a shortcut (list of
shortcuts, microphone panel). `ui/widgets/color.rs` is the compact colour picker (`color_button`,
`color_panel`, RGB / HSV / hexadecimal conversions) and the eyedropper it shares
with the application (`app/color_picker.rs`).
`ui/widgets/workspace.rs` shares the navigation, the detail panels, the
dialogs and the rows of the user inventory and of object contents
(density, hover, selection and name style). `build/ui/compact.rs` arranges
building in six sections in a narrow inspector.
`ui/color_picker.rs` composes the shared picker (RGB / LSL / Hex, HSL,
previews, palette of 32 slots and eyedropper); `ui/color_picker/model.rs`
keeps the conversions and serialised preferences, `demo.rs` the offline
gestures.
`build/ui/color.rs` captures the colours per object and face, applies the
previews and restores only the RGB on cancel. The intermediate sends are
grouped in `build/edits.rs` until release. `app/color_picker.rs` routes the
eyedropper to `Scene::color_at_ray`: candidates of the spatial index, real
triangles in parallel and tint of the face, with no GPU readback and no
effect of the lighting. It does not change the selection.
The picker keeps the identity of its calling window: placement against its
right edge, aligned at the top, or on the left if space is lacking,
recomputed at every move of the window or change of screen size. Its
measured size keeps the commands visible and moving it keeps its collapsed
state.
Moving the picker detaches it; on release, a proximity of 10 points to a
docked position on the left or on the right makes it follow again. The
chosen side stays stable as long as it fits on screen. The link to the
original colour stays present when the windows are independent. "Apply" is
checked by default, then the last choice is serialised with the saved
palette.
The swatches pass on their visible rectangle on every pass: a gradient cone
links them to the picker while it is moved separately, with no additional
interactive surface. A second click on the same swatch closes the
transaction; another swatch restores the previous one before opening.
`cursors.rs` keeps the Tabler eyedropper cursor in memory, with the tip as
the sampling point and the contrasted outline of the skin, like the cursors
of the HUDs, of the Alt camera and of the click actions.
`aurora-render/src/ui_glass.rs` produces a frosted background shared
by the visible windows: copy and two blur passes on the GPU, then
sampling within their rounded rectangles only. The resources are
reused as long as the size and the scale stay stable, then freed when
no surface needs them. The world around the windows stays sharp. A skin
with `panel_alpha = 255` keeps opaque windows without these passes. A
GPU surface that does not allow the copy keeps the tinted frame; the markers
stay undrawn to preserve the indices of the blur of the modals.
What belongs to the world (name tags and name plates, floating texts,
destination beacon) passes in front of no element of the interface: it is
painted first, with `ui::hud::world_painter`, before the bars, and ends
with the `aurora_render::end_world_labels` marker
(`aurora-render/src/ui_world.rs`). The renderer draws what precedes the
marker onto the scene, then the worn HUDs, then the rest of the interface.
The name plates sample a blur of the scene alone, limited to the area they
cover; the windows and the bars, a blur of the scene with its labels and the
HUDs. A new world label is painted with `world_painter`, never on a layer of
its own.
The scene hides the name tags and floating texts behind it, as in Firestorm
(LLHUDNameTag::render and LLHUDText::render test the depth): before a label
is painted, `aurora_render::world_label_at` marks the point of the world it
stands at, and the renderer draws what `world_painter` painted with a
pipeline of its own instead of egui-wgpu's (`aurora-render/src/label_depth.rs`,
`shaders/labels.wgsl`: egui's vertices, textures and colours, each fragment
compared with the scene's depth at its pixel, the cut smoothed over the four
texels around it). A wall cuts the name of someone standing half behind it;
nothing is read back from the GPU. What is painted without a mark (the
destination beacon) is never hidden. A name tag stands half a metre in front
of its anchor (`ui::hud::tag_point`), so that its avatar's head never cuts it,
and a click on it reaches the avatar only where the tag shows.

The display of item permissions is shared by the inventory, the contents
window of an object and the Contents tab of building in
`ui/inventory/permissions.rs`: compact C·M·T indicators, granted rights in
green and refused rights in red and struck through, colours taken from the
theme's palette, detail of the current owner and of the next owner on hover;
inventory links take the rights of their original.

Flexible prims have their own simulation (`aurora-prim/src/flexible.rs`,
port of `LLVolumeImplFlexible`) and a single geometry per object. The physics
accumulates real time in double precision and advances in fixed steps
of 1/60 s, like Firestorm's `SEC_PER_FLEXI_FRAME` clock, with at most
0.2 s of catch-up per frame. The placement of the parent is interpolated at
each step. The rendering interpolates the last two physical states (one step
of delay), with lengths preserved and the base anchored to the current
placement of the parent; this interpolation never goes back into the
simulation.
The scene (`scene/flexible.rs`) computes in parallel the forces, the deformed
path, the normals, the bounds and the selection triangles; it rewrites
only the vertices in their existing GPU allocations. The wind of each
region is received through `LayerData` (two DCT patches), interpolated like
`LLWind`.
Flexible prims are in the set of active objects, even without network
movement; origin changes shift their simulation with the world.
The outline of the selected flexibles is prepared on the same worker from
the simulated vertices (`build/silhouette.rs`). The move and rotate tools
use their negative Z anchor; the box and the size handles keep the straight
dimensions, like `LLManipScale`. Changing the size or the softness remaps
the existing chain instead of resetting it straight.

Rezzing from the inventory goes through `ui/inventory.rs` (payload and release before the end of the egui pass), `app/inventory_drag.rs` (rules, group, preparation of the commands and local consumption of the non-copyable items), `Scene::inventory_surface` (triangles among the candidates of the index, terrain, without a GPU wait) and `aurora-net/src/inventory/rez.rs` (reliable `RezObject` message in the targeted region). Transparent pixels are tested with a mask of one bit per 2×2 block, prepared by the decoding tasks; invisible and rigged surfaces are discarded. The cursor keeps the result for 100 ms at most when the ray does not move; the release always recomputes the surface. Ctrl on an object sends `UpdateTaskInventory`. The demo uses a synthetic cube as the reply to the rez.

For transfers out of object contents, `ui/inventory/task_drop.rs` keeps the
hovered row's acceptance ahead of its enclosing inventory surface for the
current egui pass. `app/inventory_drag.rs` reads that feedback after all
windows, before released cargo is cleared. `cursors.rs` caches native Tabler
copy / move badges, since Windows substitutes its ordinary arrow for the
system Copy cursor; release and Escape restore ordinary widget feedback.

1. **Network → world.** `aurora-net` decodes the messages on its runtime and
   sends them as `NetEvent`; `World::apply` updates the state (objects,
   avatars, animations, chat…).
2. **Input → agent.** The keys become control flags; `AgentState` and the
   camera advance; the `AgentUpdate` is sent.
3. **World → scene.** The poses of the avatars are computed (motion
   controller in the manner of `LLMotionController`), then the drags of the
   building tools apply their local transform, after the replies of the
   simulator and before `Scene::sync`. This way, the displayed object and its
   handles follow the same size every frame, without going back to an old
   network echo.
   When the focus belongs to the selection being edited, the camera is
   computed after this drag, before the render matrices and the culling.
   The "Follow object while editing" toggle is off by default:
   focus point and framing stay fixed at the centre of the screen; during
   a move at the edge, `build/edge_pan.rs` slides the camera
   progressively along the axis or in the plane of the handle. The inverse
   projection takes the perspective into account; the camera advances
   before the move solver, which applies the same translation to the object
   in the current frame. Their distance therefore does not drift while the
   cursor is held at the edge (no effect on HUDs).
   When on, the toggle follows the local transform. The projection of the
   cursor is then kept at the start of the drag
   so that the camera movement does not change the object without input.
   `Scene::sync` updates the
   geometry, the textures and the GPU records of what has changed or
   moves (sets maintained by events, placement computed in parallel).
   Moves and avatars are applied every frame; the full syncs (new object,
   linked geometry, rebuilt faces) and the freeing of the removed objects
   fit within a time budget: the rest waits a few frames, the closest to
   the camera first.
4. **Scene → GPU.** The sync keeps up to date on the GPU a face table
   (one entry per draw record, with its pass) and an object table
   (bounding sphere, flags, state of the avatars). `build_lists` prepares
   on the CPU only what the GPU does not do: terrain, water, transparent
   faces sorted back to front, glow, impostors, selection and debug lists
   (only the objects that have some are walked). `aurora-render` runs the
   culling in compute (main view, shadow cascades, reflections, probe;
   two-phase Hi-Z occlusion), then encodes the passes (shadows, prepass,
   scene, water, reflections, post-processing) with indirect draws whose
   count is written by the GPU. Without `MULTI_DRAW_INDIRECT_COUNT` (or with
   `AURORA_CPU_CULL=1`), `build_lists` prepares all the lists on the CPU as
   before. The scene pass always starts from a depth that is already
   written, so as to compute the colour only of what is seen: that of the
   single-sample prepass without MSAA; with MSAA, that of a second prepass
   in its own multisampled depth (terrain and opaque faces, the very draws
   of the scene pass; the alpha-masked faces, whose edge then comes from
   `alpha to coverage`, are not in it). The
   vertices of the objects and of the terrain are computed relative to the
   camera (the objects and the terrain like the depth prepasses, operation
   for operation: the scene pass tests them against that depth, and a
   difference of one rounding would show the sky through the ground): the
   vertex shader subtracts its position from the translation of the model
   (the records stay in region coordinates), and the view matrix of the
   main camera reduces to its rotation (`eye_view_proj` in `renderer.rs`);
   at altitude, single precision is not enough in region coordinates. The
   main thread stops at the closing of the frame (`Renderer::render`): the
   rest of this step is done by the render thread from the frame packet,
   while the next frame begins (see "Threads and data ownership").
5. **Interface.** egui composes the windows on the main thread; its
   shapes leave as they are in the frame packet, and the render thread
   cuts them into triangles (`aurora-render/src/ui_frame.rs`) then draws
   them over the scene. The name tags of the avatars, the floating texts
   and the dots of the maps read the `ObjectStore`'s lists of avatars and of
   objects with text (`world/objects.rs`), kept up to date by the changes:
   no window walks all the objects every frame.

The other passes of the frame follow the same rule. The particle systems
and the attached sounds brought by object updates are picked up by two
streams of the `ObjectStore` (`take_particle_changes`,
`take_sound_changes`); the mirrors are listed by the sync like the lights;
the orientation of the bodies and the loading of the avatars read the list
of avatars.

Heavy processing (image decoding, mesh building, sounds) goes through
background tasks (`scene/jobs.rs`, rayon, at low priority) to keep the
frame smooth. What they produce for the GPU (mip chains, vertices and
indices), they write themselves into the staging memory of the renderer
(`aurora-render/src/upload.rs`): the main thread only records the copies,
the most visible first, within a budget of about 1 ms per frame
(`scene/textures.rs`, `Scene::process_results`). Upkeep is done through
queues, with no periodic pass over all the textures (eviction of a few
unused textures per frame, downloaded data to be written noted on arrival),
and the memory the main thread gives back (decoded levels, J2C data, picking
triangles) is freed by a separate thread (`Jobs::discard`): while the tasks
allocate on all the cores, freeing it in place cost several milliseconds.

## Threads and data ownership

A frame costs the longer of the two threads, not their sum: the main thread
simulates and prepares frame N+1 while the render thread draws frame N.
`AURORA_RENDER_THREAD=0` runs the same packet on the main thread
(comparison, debugging).

| Thread | What it owns |
|---|---|
| **Main** (winit loop, `App::frame`) | The world, the scene, the egui interface (up to the shapes to draw, not tessellated), the window. In `aurora-render`, the `Renderer` (`main_thread.rs`): *what* to draw — draw records and face table (`RecordStore`), object table of the culling (`CullTables`), allocators of the geometry arena, pages, slots and positions of the textures (`TextureTable`), bone palettes and skin bindings, staging memory and staging copies (`UploadQueue`), settings and window size. It creates GPU resources (creation has no order) but **never writes into the GPU queue and submits nothing** |
| **Render** (`aurora-render`, `render_thread.rs`) | The `Backend` (`renderer.rs`): *how* to draw — surface and swapchain, pipelines, render targets, uniforms of the frame, tessellation of egui's shapes (`ui_frame.rs`) and egui renderer, compute culling and its tiles, occlusion, probes, impostor atlas, readbacks (depth under the cursor, culling counters, GPU timers, screenshots). **Sole user of the queue** (`write_*`, `submit`, `present`); two helper threads finish the encoders in parallel (`helpers.rs`) |
| Frame helpers (global rayon pool, `aurora-worker-<i>`) | Nothing: they lend a hand to the main thread in its parallel loops (`frame_par.rs`), on the data it lends them for the duration of the loop. The main thread does its share without waiting for them: a helper woken too late does not delay the frame |
| Window hold (`window-hold`, `window_hold.rs`, Windows) | Nothing: asleep except while Windows holds the window, when it asks the main thread for the next frame, one at a time (it waits for each to be drawn) and does the wait of the frame limiter in its place |
| Background tasks (`scene/jobs.rs`, low priority) | Decoding, mesh building; they write into the mapped staging memory, never into the queue |
| Cache of the transcoded textures (`aurora-texcache`, `scene/texture_cache.rs`) | Writes the `.bc7` files that the decoding tasks hand to it: creating a file takes milliseconds, which the tasks do not spend waiting for the disk. Queue bounded to 256 MB; beyond that, the file is not written and the texture will be decoded again |
| Frees (`aurora-trash`: `Jobs::discard`) and texture pages (`aurora-pages`, `textures.rs`) | Free, off the main thread, the memory and the texture pages that nothing uses any more. A page is handed to `aurora-pages` only after the rebuild of the texture group that contained it; since the handles are counted, a frame still in progress on the render thread (its packet holds the old group) keeps it alive until its end, and these threads never touch the queue. `aurora-pages` also creates the pages of the streaming textures (a page that asks for a new block of video memory waits for the driver for 1 ms or more): the main thread requests the page of a size ahead of the uploads queued for it (`TextureTable::prepare_page`) and receives it ready, the upload that arrives before it waits one frame |
| Conversation history (`aurora-chatlog`, `world/chat_log/store.rs`; `aurora-transcript`, `chat_log/transcript.rs`) | `aurora-chatlog` writes the lines of the conversations into their files and reads back the end of a file when a conversation opens: no reading or writing of these files on the main thread, which drops tasks into a bounded queue and receives the histories read. `aurora-transcript` (one per open History window) reads a whole file: index of the pages, pages on demand, search |
| Network (`aurora-net`, tokio runtime) | Circuits, capabilities, downloads |

What passes from one thread to the other:

- **Write log** (`writes.rs`). `Queue::write_*` takes effect at the next
  `submit`, whichever thread calls it: a write made directly by the main
  thread for frame N+1 would leave with frame N still in progress.
  Everything the stores of the scene want to write (modified ranges of the
  CPU mirrors, geometry, texels, copies to submit in the middle of a frame
  such as the growth of an arena) is therefore recorded, in order, in the
  log of the frame under construction; the render thread replays the log of
  frame N just before encoding frame N. A frame is always drawn with the
  data of a single frame, whole, and the uploads keep their order relative
  to the draws.
- **Frame packet** (`packet.rs`), one per frame, self-contained: the log,
  the encoder of the staging copies and the chunks of staging memory to
  remap after submission, the handles of the scene's buffers as they are
  for this frame (the main thread can replace them afterwards), camera and
  environment (`FrameParams`), CPU draw lists (`DrawLists`, copied) and view
  of the GPU culling, egui shapes still to tessellate and changes to its
  textures (`ui_frame.rs`), window size / vsync / settings to apply,
  readback requests (hovered pixel, clicked pixels, screenshot).
- **Rendezvous handoff** (`render_thread::handoff`): the render thread holds at most one
  packet, none waits behind. If the rendering is late, the main thread
  waits (back-pressure) instead of piling up frames; each side measures its
  wait (`wait_render`, `rt_idle` of the profile).
- **Frame result**, returned at the next handoff: statistics of the
  rendering, reply to the hover (kept with the camera of the frame that
  requested it), replies to the clicks, culling counters, screenshot,
  buffers of the packet to reuse.

Only one thing waits for the render thread: the frame that carries a
screenshot (same frame captured as before, read just after). The depth under
a click no longer waits for it (`Renderer::request_pick`): the request
leaves in the packet of the next frame, the render thread copies the texel
before encoding that frame — the depth and the camera are still those of the
frame on which the click was made — in commands submitted separately, and
the reply comes back with a frame result, one to three frames later (the
result of a frame the render thread has already finished is taken without
waiting for the next handoff)
(`aurora-render/src/pick.rs`, `aurora-viewer/src/app/click_pick.rs`).
The same click also asks what is drawn under its pixel
(`Renderer::request_drawn`, `aurora-render/src/drawn_pick.rs`): the frame
that carries the request redraws the faces of the view through a camera
tightened on this pixel and its neighbours, with the vertex shader of the
scene, into a target of 2 × 2 texels (draw record, depth, surface
coordinates): opaque and masked faces, then the transparent faces in front
of them. Nothing is drawn or allocated per frame outside a click.
The old read, which waited for the frame in progress then for everything the
GPU had queued (`Renderer::pick_world`), is now only used for comparison
(`AURORA_CLICK_PICK=wait` or `check`). If the render thread disappears
(panic), every wait ends, the pending clicks receive an empty reply and the
viewer closes cleanly.

Worn HUDs go through `scene/hud.rs` (lists of our avatar, interest of the textures, triangle picking) and the shared projection `aurora-render/src/hud.rs`. The attachment points in `Scene::object_transform` are relative to the screen, with the aspect of `mScreen`, without the position or the orientation of the avatar. The renderer draws the opaque / masked then the transparent faces after post-processing, in an independent depth, before egui; it reuses the geometry, the bindless textures, the materials and the UV animations of the world. `demo_hud.rs` provides the offline updates and touch replies.

The editing of HUDs reuses the building tools: `build/geom.rs` adapts their camera to the orthographic HUD projection (parallel rays, handles of constant size in pixels), `build/mod.rs::edit_parent` provides the transform of the attachment point to convert the moves into local coordinates. HUD and world selections stay separate. `build/manip.rs` sends the local positions on release with the linkset flag; the Object tab also uses the local values for HUDs. Direct moving with the chosen key (ALT by default) shares this same edit_parent and the picking of the visible faces, with the double-sided exceptions. Preferences › Shortcuts › HUDs uses `ui/options.rs` and the `keybinds::Action::HudDrag` action: the two keys share the capture, the clearing, the conflicts and the reset of the other shortcuts. Modifiers alone are captured on release and their left / right sides are equivalent. `Settings::sanitized` migrates the old `hud_drag_key` to the main key without overwriting the two choices already saved; `app/hud_drag.rs` uses these shortcuts for the gesture and the cursor.

What our avatar wears on its body is edited in the same way: `build/mod.rs` distinguishes three selection spaces (`SelectType`: world, worn object, HUD, never mixed, like `LLSelectMgr::canSelectObject`), `edit_parent` gives the reference frame of the attachment point as it is posed at that instant (animated bone included), and the positions typed or dragged are sent in that reference frame. With the "World" grid, the arrows follow that reference frame (`LLSelectMgr::getGrid`). A worn object cannot be deleted, taken, copied or linked and does not enter a selection rectangle; those of the other avatars stay unselectable. "Touch" and "Edit" in the menus of the inventory (`ui/inventory/context.rs`) and of the outfit (`ui/appearance/items.rs`) go through `CtxAction::TouchWorn` / `EditWorn`, resolved by `World::worn_attachment`. `demo_worn.rs` and `app/worn_demo.rs` provide the worn object and the offline scenario (`AURORA_DEMO_WORN`).

"Inspect Objects" (`ui/inspect.rs`, port of LLFloaterInspect with Firestorm's additions) lists the prims of the clicked object: `app/inspect.rs` requests their properties through ObjectSelect / ObjectDeselect (without touching the selection of the building tools), assembles the rows every frame and outlines the prims in the world; `scene/inspect.rs` gives faces, vertices and triangles at the displayed level of detail, the textures and their memory, and the complexity of the linkset with the rules of `scene/complexity.rs`. The chosen columns are saved (`Settings::inspect_columns`, FSInspectColumnConfig).

The HUD faces turned away from the screen are rejected in `fs_hud`, before any colour or depth write. The `DOUBLE_SIDED` material flag keeps the explicit glTF exception without breaking the order of the translucent faces. HUD picking follows the same triangle winding and the double-sided materials / overrides; the invisible back faces do not intercept the visible buttons.

The HUD projection and lists are part of the copy of the frame packet (`DrawLists::copy_from`): the render thread owns its own copy while the main thread keeps the projection for picking.

The world menus share a model of entries in `ui/context/entries.rs`
(actions, rights, submenus, check marks). `ui/menu.rs` draws the existing list;
`ui/context/pie.rs` lays out these same entries in eight sectors, with
Firestorm's directions and "More" submenus for the additional actions. The
sectors that are not hovered take the transparency of the theme's windows.
Left click selects; right click goes up one level (closes at the root). The
navigation path stays in the open menu and is reset when it closes.
`Settings::use_pie_menu` keeps the choice of Preferences › Interface; the list
remains Aurora's initial choice.

## Data on the machine

The local zoom specific to each HUD is held by `world/hud_scale.rs`: `hud_scales.json` profiles in the account folder, stable identity by inventory item UUID, with the attached object name and creator shared by new copies and temporary experience HUDs. Full ObjectProperties are requested with batched ObjectSelect / ObjectDeselect messages, preserving the user selection; failed requests retry at most once per ten seconds. Inventory labels remain legacy aliases, and inherited inventory fallback sizes yield to the common object identity when metadata arrives. Aliases are persisted and forgotten with the shared profile. Each inventory item retains its own size after reattachment or renaming, including before creator metadata arrives; old name / creator profiles are read and bound to items as they return. Writes are atomic on a helper thread and worn copies remain independent. Forgetting removes the selected item profile and the shared fallback while preserving the current visual size and other item profiles. `app/hud_drag.rs` reserves the wheel with the move key, draws the border / percentage and keeps a part of the HUD reachable. While editing with a HUD selection, the wheel without this key changes only `World::hud_zoom`, a projection common to all the HUDs, like LLAgentCamera; it comes before the media and the camera even in an empty area. This view zoom does not change the local factors or offsets. The root keeps its origin on screen; the linked offsets and the placement / picking matrices follow the uniform factor. The building tools convert the linked positions back, and the touch information sent to the server gets its original coordinates back. Server dimensions and positions are not changed by the zoom; the possible offset that keeps the HUD reachable stays local.

### Crash recovery

`aurora-viewer/src/crash/` keeps one folder per session (UUID), before
the initialisation of the network and of the rendering. `mod.rs` manages the
snapshots (version, commit, language, theme), the Rust trace and the recovery
at the next launch; `native.rs` installs the Windows exception filter and
watches the end of the process through a system handle, with a check of its
creation date to avoid the reuse of a PID. The watcher is the same
executable, started in internal mode before any initialisation of the viewer.
Its working folder is that of the reports, to let the updater replace the
installation folder after the viewer has stopped. It never relaunches the
viewer.

`window.rs` reuses Fluent, the Palette, the fonts, the Tabler icons
and the buttons of the viewer. `software.rs` draws its egui meshes on the
CPU and `softbuffer` presents the pixels, without initialising Vulkan or any
graphics adapter. The loop waits for events between frames.

`report.rs` composes the draft from this snapshot and the markers of
the stopped session. "Send a crash report" opens the same
`ui/bug_report.rs` as the viewer, with its checkboxes, its preview and its
GitHub sending modes. The form receives the folder of the session; it does
not collect the configuration or the log of the recovery process.
The workers read the filtered excerpt and prepare the files while
the window waits for events. The optional private ZIP also contains
the Rust / native traces, never in the public text of the issue.

The log is also written in the session's own folder: neither
the rotation of the ordinary log nor another instance can replace
it. The full traces and logs stay local. A file lock
prevents two windows from presenting the same report; a single
automatic recovery attempt is made at the next launch if
the first window could not open. Confirmed reports stay
on disk; the folders of the sessions that ended normally are removed, by
their watcher or, when it could not start, at the next launch.

Immediate watching and native capture concern Windows
(the platform of the releases). A native trace contains the exception code,
the faulting address and the addresses of the stack; a Rust trace contains
the available symbols. A damaged stack or a forced stop may
leave no trace, which the window explains explicitly.

The reports are under `logs/crashes/sessions/<uuid>/`, and the offline
trials under `logs/crashes/demo/<uuid>/`; `session.json`, `session.log`,
`panic.txt`, `native.txt` and the markers belong to a single
run. The internal arguments `--crash-watch` and `--crash-show`
take the UUID of a session, never a path: the helper process opens
only the real folder (not a link) of that name immediately under its
own root. In the offline demo the viewer hands it that root through
`AURORA_DEMO_CRASH_DIR`, as an absolute path.

| What | Where (Windows) |
|---|---|
| Settings, skins, window layout | `%APPDATA%\Aurora\AuroraViewer\config\` |
| Files of an account (contact sets, pinned groups, teleport history, HUD zooms) | `%APPDATA%\Aurora\AuroraViewer\config\accounts\<account uuid>\` |
| Conversation history (readable text, one file per conversation; never attached to a bug report) | `%APPDATA%\Aurora\AuroraViewer\config\accounts\<account uuid>\chat_logs\<grid>\` (`agni`, `aditi`, `custom_<login address>`; the demo: `config\demo\accounts\<uuid>\chat_logs\demo\`) |
| Cache (textures, mesh, animations, inventory…) | `%LOCALAPPDATA%\Aurora\AuroraViewer\cache\` |
| Logs (`aurora.log`, `aurora-demo.log`, `aurora-demo-<title>.log`) | `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\` |
| Remembered password (hash, never the password) | Windows Credential Manager, "Aurora Viewer" service |
