# Aurora Viewer architecture

Object particles sample the containing region's `WindGrid` at each live
particle's render-space position through `scene/particles.rs` and
`aurora_prim::particles::ParticleSource::update_with_wind`. Only `LL_PART_WIND_MASK`
particles pay for the lookup. Region offsets, neighboring grids and particles
outliving their emitter share this path. Outside the known regions the main
region supplies wind, as in the Legacy Viewer; without a main grid it is zero.
The grid owns Legacy interpolation/scaling, and the simulator retains the
Legacy velocity relaxation before target and acceleration updates.

Native links share `ui/chat/action_marker.rs`: a parsed action chooses its
Tabler icon before the unchanged destination text. Wrapping text
and console/bubble LayoutJobs share that prefix; the marker is part of the
link's hit area while transcript selection keeps the original message text.
The reserved glyph uses the surrounding text's metrics, preserving the
sentence baseline and line height. Unknown commands use a localized readable
fallback; their actual URI stays available through the copy-link menu.

RLVa is an experimental, explicit opt-in (`Settings::rlva_enabled`, false by
default), exposed in Preferences > Protections with the session's active
restrictions. `world/rlva.rs` owns parsing, object-scoped rules, strict
exceptions, protocol replies and garbage collection. Its `policy.rs` checks
the common action boundary; `inventory.rs` resolves #RLV paths and validates
COF deltas, attachment/layer locks and scripted nostrip. `camera.rs` supplies
temporary camera settings and exclusive ownership. `events.rs` handles
permission/teleport consent, command notifications and stand returns;
`editing.rs` checks linksets and material/media writes. `environment.rs`
keeps scripted sky, water and day cycles separate from saved local choices.
`sitting.rs` records actors only at accepted native sends, observes confirmed
seat transitions and retains the previous parent for stand notifications.
A forced ground sit leaves an object seat first, then rechecks the action
after standing; disabling or a stand-return teleport cancels the delayed sit.
`wardrobe.rs` compares complete COF clothing/bodypart snapshots only when
inventory generation changes. It skips the initial baseline and incomplete
fetches, then emits all wearable removals before additions.
`groups.rs` resolves forced group/role changes through native membership and
role data, retains the actor across asynchronous replies and applies the
reference's 60-second throttle and single early change for the lock owner.
Simulator-triggered group changes restore only the last permitted group;
the internal restoration actor cannot activate an arbitrary group.
`privacy.rs` provides display-only identity/location projections, keeping
canonical cache records, transcript file identities and region coordinates
intact. `privacy/matcher.rs` compiles Unicode word-boundary replacements into
a shared trie, reused while the patterns stay unchanged. Name exceptions,
limited avatar menus, location writes, application links and teleport offers
are checked at both their native UI paths and the common action boundary.
The current teleport-history label follows location privacy without losing
its navigation coordinates; persisted arrivals omit a hidden current place.
`effects.rs` owns per-object sphere/overlay modifiers, tween targets and
overlay linkset arbitration. `aurora-render/src/rlva.rs` runs at most six
depth-aware sphere effects after glow, tonemapping, antialiasing and scene
selection, with two reusable composed-scene targets; separable blur
uses distinct per-pass uniforms. No effect means no targets or GPU passes.
`rlva/overlay.rs` composes bindless overlay textures after world labels and
before HUD attachments, including photos without an interface. The native
texture streamer also supplies a compact CPU alpha mask for object clicks;
terrain navigation, HUDs and transparent holes remain usable.
`watchdog.rs` tracks attachment items across object replacement, restores a
locked lost inventory item after SaveAssetIntoInventory (15-second fallback,
30-second retries), and rolls back a wear whose actual point is locked.
Replacement adds first, then removes only unlocked predecessors, preserving
locked COF links. Its internal actor authorizes only the exact pending item
and point or rejected predecessor. `temporary.rs` applies point/source locks
to attachments with no inventory record; `aurora-net/src/attachment_detach.rs`
addresses ObjectDetach to their simulator. Disabling clears all recovery work.
`debug.rs` validates allowlisted writes and other issuers' setdebug locks.
The render-resolution divisor resizes the actual 3D targets, including
depth, occlusion, TAA/SMAA, DoF and post processing. `rlva/resolution.rs`
upscales their composed result before world labels, overlays, HUDs and UI;
depth picks map physical window pixels to the scene's actual dimensions.
HUD selection keeps a separate full-resolution mask. The ordinary path
allocates no reduced-scene target and runs no upscale pass.
Script replies use `NetCommand::ChatReply` and `aurora-net/src/chat_reply.rs`:
one reliable shout, including an empty response, UTF-8 truncated to the
protocol's 1023-byte limit. They bypass user chat splitting and redirection.
`app/rlva.rs` dispatches
forced actions through the native application handlers. Camera constraints
also cover the free camera; HUD text and avatar tags are filtered at drawing.
Disabling clears rules, queued commands and temporary camera values. No
restriction is saved to settings or restored for a later account.
Forced outfit transactions retain their issuer through the asynchronous COF
reply and attachment sync. A later query waits until the preceding outfit
transaction finishes. Inventory moves/copies compare lock sources, and
renaming or removing a parent cannot invalidate explicit folder locks.
`app/teleport_lookup.rs` retains the issuer, destination and orientation of
named teleports behind a single request ticket. Map resolution only returns
that ticket; the app sends the resolved teleport through the common guard.
Superseded replies are ignored. Maturity deferrals also retain the issuer;
disabling cancels delayed script teleports while preserving user requests.
`rlva/naming.rs` handles balanced attachment-name blocks and legacy folder
syntax. `collection.rs` is shared by forced wear and worn-inventory queries:
it folds point folders and single-level folder links, includes gestures,
honors shared-wear and prefix preferences, and reports missing targets for
asynchronous fetching. Query counts use one ancestry walk per collected item.
`preferences.rs` persists user choices separately from session restrictions.
`rlva/im_queries.rs` handles online version/stop/list/exception requests after
mute/friends-only checks and before IM receive restrictions. Listing uses the
native notification consent path, with an explicit remembered preference.
`aurora-net/im_reply.rs` encodes bounded busy/normal responses independently
of user IM locks; disabling drops pending responses and permission questions.
`rlva/rename.rs` observes confirmed new attachment arrivals in legacy shared-wear
mode. It appends a point name or creates/reuses a folded point folder, waits
for metadata confirmation, bounds retries and preserves later manual renames.
Disabling cancels its work; already-sent inventory replies still update the
ordinary inventory model without restarting the naming workflow.

This does not yet provide full RLVa parity. [RLVa implementation status](docs/RLVA.md)
lists the supported commands and remaining families; versionnum deliberately
returns 0 until complete protocol compatibility can be claimed.

Explicit profile openings restore their destination through the shared floater
expansion, following `LLFloater::openFloater`: avatar profiles (both layouts),
groups, Places/standalone place details, experiences, payments, abuse reports
and Marketplace. URI commands use the same inventory/search/map openings for
all text sources. `ui/uri_restore_tests.rs` renders real destination windows
to check reopening and intentional minimization between requests.

Notification identity names carry typed avatar/group/experience/object targets in
`world/notifications.rs`, retained by `world/notification_history.rs` across sessions.
`ui/notifications/text.rs` styles and hit-tests their exact text spans with the
chat's glyph-range geometry, dispatching the existing native profile/SLapp actions.
Cards, presence toasts, friendship prompts and history share those links; clicking
a name leaves the notification's answer pending. Place links retain their existing
chat renderer and region badges.

Attachment frames follow `LLViewerJointAttachment` and `LLXformMatrix`: a bone scales its attachment joint's local offset, while rigid object sizes and linked child offsets retain their own scales. `scene/avatar.rs::AttachPoint::frame` shares that rule between CPU/GPU anchors and automatic hover; appearance hints use the same absolute bone frame. `scene/joint_overrides.rs` selects joint positions and nonzero pelvis fixups by greatest mesh asset UUID, independently of object arrival/traversal order. Scale locks from every qualifying mesh remain independent of the winning position. The pelvis is converted from root-relative Legacy coordinates to Aurora's feet-origin skeleton, without canceling its local translation in the root height correction. Animation position tracks still set absolute local positions, including the pelvis. Ground control avatars receive their own pelvis fixup; worn control avatars cannot deform the wearer.

Rigged mesh compatibility uses `scene/anim.rs::Rig::skin_joint` to resolve
skin names and aliases, falling back to the pelvis for unknown names as
`LLSkinningUtil::initJointNums` does. World skin bindings, appearance hints
and automatic foot measurements share that rule and preserve each mesh's
inverse bind matrix. Vertex joint indices are bounded to the mesh palette
before remapping, following `LLSkinningUtil::getPerVertexSkinMatrix`.

## Scope

`world/anti_spam.rs` owns session-local counters and source/category blocks,
following NACLAntiSpamQueue::checkEntry and the incoming message handlers in
`indra/newview/NACLantispam.cpp`, `llimprocessing.cpp` and `llviewermessage.cpp`.
It filters chat text, prompts and offers before notification/attention work and sounds
before asset requests or playback. PreloadSound events retain object and owner
UUIDs (`aurora-net::SoundPreload`). Newline floods follow the Legacy Viewer's
LF threshold and shared IM block, including owner aggregation for nearby chat.
Scripted inventory offers identify their owner; the own-object option includes
those sent by our avatar's objects. Settings persist; blocks do not. Preferences
› Protections (`ui/options/pages/protections.rs`) resets counters or unblocks one source.
Its Cx rows participate in preference search, modified settings and page reset.
`demo_anti_spam.rs` supplies synthetic floods through the production event path.

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

The build mesh inspector (`build/ui/mesh.rs`) reads the actual cached LOD
geometry through `scene/mesh_inspection.rs`. It retains only the inspected
mesh's four asynchronous geometry requests and a transient per-object LOD
override shared by full sync, parallel planning and LOD checks. Neither the
inspection choice nor the informational distances are simulator properties.
Override changes set the local render invalidation as well as the dirty queue:
the frame scheduler discards dirty entries whose render state is unchanged.

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
│  └─ emoji/                    Noto Color Emoji font and its licence (built into the exe)
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
| `aurora-net` | Login (XML-RPC), UDP circuits, capabilities, event queue, HTTP download of assets; runs on its own tokio runtime. `probe.rs` (AURORA_PROFILE): counters by UDP message type, by kind of object update and by kind of HTTP request and asset host, off behind one flag. `transition.rs` (AURORA_PROFILE): the timeline of a teleport or a region crossing, one log line when it ends, and the `region link` lines of the neighbour circuits. `agent_update.rs`: when an AgentUpdate leaves (a change of the control flags at once and confirmed, the rest past the Legacy Viewer's thresholds at 125 a second at most, one a second to stay alive); the viewer hands the session what must leave before its next tick (`NetClient::set_controls`). `throttle.rs`: the bandwidth asked from the simulator, port of LLViewerThrottle (the setting shared between the seven channels, lowered while packets are lost or the viewer falls behind). `objcache.rs`, `session/object_cache.rs`: per-region object cache (the viewer side of ObjectUpdateCached) and its files kept off the session loop: read by a blocking task before RegionHandshakeReply leaves, written from snapshots by one writer. `session/object_actions.rs`: touch / grab / release and object inventory (capability or memory-bounded UDP transfer); `task_inventory.rs`: LLSD parsing and legacy file; `experience.rs`: experiences' details (GetExperienceInfo), the agent's permission for each (ExperiencePreferences) and the experiences a group owns (GroupExperiences); `land.rs`, `session_land.rs`: parcel messages ("About Land", RemoteParcelRequest of any place); `estate.rs`, `session/estate.rs`: the Region / Estate window's RegionInfo, EstateOwnerMessage `getinfo` with its invoice, EstateAccess / RegionExperiences / RegionSchedule / ExtEnvironment and SimStats, and its "Apply" (DispatchRegionInfo or `setregioninfo`, EstateChangeInfo or `estatechangeinfo`, `setregionterrain` / `texturedetail` / `textureheights` / `texturecommit`, the terrain's bake), and the estate lists' `estateaccessdelta` with the `setaccess` answer that asks them again; `group_money.rs`: a group's parcels (PlacesQuery / PlacesReply, also through the event queue), our land contribution (SetGroupContribution) and its accounts by week (GroupAccountSummary / Details / Transactions); `home.rs`, `user_info.rs`, `session/account.rs`: Set Home to Here (HomeLocation capability or SetStartLocationRequest, the simulator's home alerts) and the account's directory settings (UserInfo capability or UserInfoRequest / UserInfoReply / UpdateUserInfo) |
| `aurora-msg` | SL's UDP message system: packets, zerocoding, typed messages generated from `message_template.msg` (build.rs) |
| `aurora-llsd` | The LLSD type and its XML / binary / notation formats |
| `aurora-prim` | Prim model: volume parameters, faces, extended parameters, geometry generation (port of `llvolume`), Linden trees and grass (`tree.rs`, `grass.rs`: species tables and meshes) and the lattice noise they share with the terrain (`noise.rs`) |
| `aurora-assets` | Asset decoders: JPEG2000, mesh, animations, materials, `.llm` avatar meshes, skeleton |
| `aurora-render` | wgpu / Vulkan rendering engine: bindless textures grouped into pages (texture arrays), sub-allocated geometry, multi-draw-indirect, shadows, reflections, post-processing; texture animations (`tex_anim.rs`: CPU reference and parameters of the records, evaluated by the vertex shaders); GPU-driven draw lists (`gpu_cull.rs`, `shaders/cull.wgsl`: face and object tables maintained by the scene, culling and compaction in compute for each view, drawing through `multi_draw_indexed_indirect_count`); local lights sorted by screen tile and by depth (`light_tiles.rs`: for each column, row and depth slice of the main view, the set of lights that cover it, computed every frame on the render thread; a fragment only walks the lights common to the three, `light_walk` in `shaders/common.wgsl`, instead of the 64 of the list; the reflection probes of a fragment are found the same way, in sets of their own, `probes::tiles` and `probe_tiles`); two-phase Hi-Z occlusion (`occlusion.rs`, `shaders/occlusion.wgsl`, shared test `shaders/hiz_test.wgsl`); ambient occlusion, the Legacy Viewer's (`shaders/ssao.wgsl`) or GTAO after Intel's XeGTAO (`gtao.rs`, `shaders/gtao.wgsl`: depth levels, horizon search, accumulation over the frames (reprojected history with depth rejection) and edge-aware blur in compute, its result where the Legacy one goes, with its bent normal for the specular occlusion; optionally the visibility bitmask with one bounce of the previous frame's lit colour, the screen-space indirect light, `main_bitmask`); contact shadows of the sun after Bend Studio's screen space shadows (`contact_shadows.rs`, `shaders/contact_shadows.wgsl`: wavefronts of 64 pixels along rays towards the light's point on screen, from a copy of the depth prepass into which the blended faces opaque enough and the impostor cards are drawn, the darker of them and the shadow map taken by the shading); geometric specular anti-aliasing of the objects' highlights (`specular_aa_roughness` in `shaders/common.wgsl`, its math mirrored and tested in `specular_aa.rs`); Aurora's bloom beside the Legacy glow (`bloom.rs`, `shaders/bloom.wgsl`: levels of the light over a threshold, extracted in `shaders/post.wgsl`, downsampled and added back up, spread by the post pass); the automatic exposure of skies that are not classic (`exposure.rs`, `shaders/exposure.wgsl`: one compute workgroup averages the luminance of the middle of the scene and eases the exposure towards its target, copied into the post pass's uniforms without a read back); shadows drawn, in each cascade, with only the objects whose shadow can fall on what the view reads there (`shadow_cull.rs`: volume of the pixels that read the cascade, stretched towards the light, tested by the GPU culling and the CPU lists; opaque faces without a fragment shader), and an opaque object drawn in one cascade only, the next cascade taking that part of its map from the finer one (`shadow_merge.rs`, `shaders/shadow_merge.wgsl`); streaming uploads with no copy on the main thread (`upload.rs`: mapped staging memory that the background tasks write into, GPU copies recorded in an encoder submitted at the head of the frame); textures compressed to BC7 when they support it without damage (`bc7.rs`: Intel ISPC encoder and quality check of each level; pages are in RGBA8 or in BC7 depending on the format of the chain, `TexelFormat`); nothing that grows or is freed all at once within a frame (`arena.rs`: geometry ranges allocated best-fit through an index of sizes, draw records kept in blocks and GPU buffer grown by a GPU copy; `textures.rs`: shrinking a texture by copying its levels to a smaller layer, grey substitute shared by the pending textures, emptied pages destroyed a few per frame by a separate thread); allocations that can fail when video memory is full (`gpu_alloc.rs`: a growth that the driver cannot make keeps the buffer it had and is tried smaller or later, never installed invalid; repeated wgpu errors written once with a count); presentation mode of the swapchain (`present.rs`: a single rule at the creation of the surface and at every toggle of vertical sync, mode forced by `AURORA_PRESENT_MODE`); two halves, see "Threads and data ownership": `main_thread.rs` (the `Renderer` as seen by the scene: its stores, the closing of the frame), `writes.rs` (log of the frame's GPU writes), `packet.rs` (frame packet and result), `ui_frame.rs` (egui shapes handed to the render thread, which tessellates them like `egui::Context::tessellate`), `thread_cpu.rs` (CPU time of a thread, for the profile), `render_thread.rs` (render thread, rendezvous handoff of the packets), `renderer.rs` (`Backend`: surface, pipelines, targets, passes), `helpers.rs` (helper threads of `finish`) |
| `aurora-audio` | Audio output: mixer with the SL volume channels, music streams, world sounds, wind noise |
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

`ui/fonts.rs` retains unscaled font definitions and installs the combined skin
and Preferences text scale on every face. egui's shaped advances and row
metrics therefore scale explicit `RichText`, painter fonts, semi-bold,
monospace and system fallbacks as well as standard text styles. Reinstalling
fonts or changing size starts from the unscaled definitions. Startup restores
saved window state before the theme and fonts so the scale survives reopening.
Inline emoji images follow the same scale; menus and name plates allow scaled
row heights.

Preferences measures its page header before allocating the remaining scroll
viewport. The categories have their own bounded viewport above the fixed
reset button, so larger fonts and shorter windows cannot drive height growth.

`ui/hud.rs::NameTagVisibility` shares self/friend/other visibility and the
Legacy Viewer temporary clock between classic tags and Aurora plates. A
continuous visible interval ends with a one-second fade; leaving the view
prunes the clock so returning starts again. Filtering precedes both painting
and hit rectangles; object floating text keeps its independent lifetime.

Preferences › Aurora › Animations selects the playback clock for both avatars
and animesh. Legacy is the persisted default, including older settings files:
activate at asset loading and suspend playback during editing, following
LLMotionController::updateLoadingMotions and pauseAllMotions. The opt-in
Synchronized clock uses the first locally known simulator signal as its origin
(`world::PlayingAnimation`), independent of asset loading and first visibility.
In this mode, `scene/anim.rs` freezes only the displayed pose during editing;
receipt origins remain unchanged, so releasing the selection rejoins the current phase. Changed
signals and late assets cannot move the held edit pose. Late assets join the
elapsed phase and expired one-shots report completion once. Animesh
linkset merging preserves the receipt clocks of the contributing signals.
This intentionally improves on activation at asset arrival in
LLMotionController::updateLoadingMotions;
it cannot recover a simulator start that the viewer never received.
Switching to synchronized playback rejoins the receipt phase immediately;
switching back preserves the current phase and uses Legacy timing for future
starts and edit pauses. Import preview seeking overrides either preference.

The same section's disabled-by-default `Settings::ground_avatar_feet` controls
terrain adaptation in `scene/anim/grounding.rs`, applied to the final blended
pose without feeding corrections back into the source animation. Additive
procedural rotations are included in the source pose before placement;
the held blend state and live expression/hand weights retain their own updates.
The previous
placement solver was replaced at the human's request. The new implementation
separates authored foot clearance, terrain elevation and world-space body
elevation, following independently implemented concepts from
[ozz-animation Foot IK](https://guillaumeblanc.github.io/ozz-animation/samples/foot_ik/)
and [Daniel Holden's foot IK](https://www.theorangeduck.com/page/inverse-kinematics-foot-locking).
`Settings::ground_feet_max_height` is a saved 0–1 m slider beside that switch,
defaulting to 0.4 m. It bounds each foot's vertical terrain offset relative
to animation and the accompanying body offset, including retained smoothing
state when reduced live. Support traces cover the full one-metre range.
`Settings::ground_feet_while_moving` is off by default: walk/run/crouch-walk
signals, physical motion and the drawn stopping glide release placement through
its existing fade. Current input and the AO's movement state cover our avatar
before a velocity update; turning in place retains support. Enabling the
setting keeps placement active during grounded movement.
An owned native Movelock ground sit with its verified standing AO actually
masking the sit is treated as standing. Only that context supersedes stale
airborne signals or velocity; manual ground sits, chairs and flight remain
excluded. The application supplies this context anew with every AO update.
For large corrections, the body constraint keeps an upright source hip above
its planted ankle so that the knee hinge cannot turn backward.

Four shaped sole probes use the existing neutral shoe calibration. Source
clearance with hysteresis distinguishes contact and flight, measured before
the render-space translation to retain submillimetre precision. Contact targets
follow the support; a flying foot retains the source arc from its departure
elevation, transitioning toward the landing elevation during descent. Collision
only raises flight targets. A critically damped terrain-offset filter eases
changes toward lower support without delaying the current source clearance;
absolute elevation absorbs collider-root steps. A final upward projection
prevents penetration during that transition. There are no horizontal anchors: authored lateral
trajectories remain unchanged. Support tilt preserves authored heel/toe roll.

A single height filter follows absolute world elevation, absorbing physical
root-height changes before they reach the rendered body. Without any known
support, source body motion passes through unchanged. Initial placement
starts at the valid support height. The common support
height raises a body compressed over two higher supports; both contact-leg
reach intervals bound its displacement within the configured height. Down-step anticipation acts
on body elevation using the whole future footprint, limited to stair-size
drops (25 cm); a ledge must not provoke a crouch before support is lost.
Analytic two-bone IK uses
the knee's rig-defined local Y hinge and retains the source lateral knee offset
in metres while changing forward flexion. Segment lengths and authored bends
survive small contact corrections; precise rotation arcs retain submillimetre
adjustments without a near-parallel identity dead zone. Near-straight joints
cannot amplify their inward taper. Priority 5/6 legs or pelvis, control avatars, seating, flight and import
previews keep control. Editing pauses the placement clock; teleports and shape
changes reset it. Foot/body elevation and support normals settle over about
500 ms; disabling fades only this pass's pose deltas in 500 ms;
enabling seeds terrain offsets and body height from the uncorrected pose or
the visible fade pose, including after a completed fade, instead of jumping
to the full correction.

`scene/grounding.rs` performs short solid-geometry/terrain traces on the parallel
pose workers, with region-corrected simulator contact as a streaming fallback.
`demo_grounding.rs` authors walking/recovery from explicit source foot paths,
independently of terrain adaptation, and seeds position once before leaving
normal controls free. Slope, steps, elevated supports, lift, movement and disabled
comparisons expose contacts and transitions; frontal views show both knees.

Object and attachment menus offer Object information immediately after Script
information, without a preference. `world/bridge/services.rs` correlates these
separate requests and formats the extended `llGetObjectDetails` report in nearby
chat, retaining the clicked prim's UUID and the basic script summary.
`aurora-net/src/bridge/query.rs` validates the bounded typed response; bridge
protocol 1.21 adds this read-only query without changing movement commands.

`ui/land/scripts/table.rs` caches the script-resource rows by the world's
resource revision, sorts numeric columns by their values and filters object
or owner names. Late avatar/group name replies refresh the search and owner
ordering. Each tab keeps its own search and sort; only visible rows are laid
out, and filtered-out objects leave the action selection.

`ui/widgets.rs` gives the shared floaters and mini-map a click-and-drag
background response: egui's click tolerance delays movement while the pointer
can still be clicking, with child controls retaining priority. Deliberate
movement keeps egui's existing drag, constraints and saved positions.
`ui/widgets/movement_tests.rs` exercises pointer jitter, button activation and
complete drags on expanded, fixed, inspector, minimized and bare windows at
three interface scales.

Notecard previews show item descriptions above all three views. Description edits
use inventory metadata mutations or `UpdateTaskInventory` for object contents,
independently of the content draft and its asset upload. Modify permissions
apply even when the content itself cannot be read or edited.

`ui/inventory/markdown.rs` provides each inventory/object notecard's Classic,
Markdown and split Edit views. Classic retains Firestorm's plain proportional
text with embedded inventory. CommonMark is parsed once per source change on
the worker pool and displayed as a selectable, read-only rich text editor.
Edit places the raw source on the left and its live Markdown preview on the
right, with independent scroll areas and a common save footer.
`InventoryPreferences::notecard_view` persists the last explicit tab choice
for inventory and object notes. New document windows inherit it on first
display; existing windows retain their own mode and draft. Old settings
without the preference start in Classic.
`ui/inventory/markdown/formatting.rs` adds the Edit toolbar and syntax help;
formatting transforms the source selection, preserves Unicode and embedded
markers, and records one native undo step per action. The latest
ready preview remains visible while a changed draft is being parsed;
the existing embedded-item renderer keeps attachment actions in every view.
All modes share the original source draft and permission-aware save lifecycle
in `ui/inventory/document.rs`; formatted text never enters the asset encoder.
`ui/inventory/markdown/code.rs` maps fenced LSL tokens back to the exact source
and formatted preview. The document worker uses the script editor's lexer;
drawing resolves the same current skin/custom script colors. Edit reuses
`ui/scripts/indent.rs` and its selection commands, with LSL scopes for comment
shortcuts, automatic indentation and brace matching. Tab stops and multiline
Tab/Shift+Tab work throughout the source; pasted text remains untouched.
Code-block backgrounds use character ranges from the parser and the shaped
text rows: one full-width surface per block, below native text/selection,
including wrapped and empty rows. Inline code retains its glyph highlight.

`ui/texture_library.rs` owns the compact lateral grid and bottom shelf, their
shared folder/search/type state and inventory management shortcut.

`ui/inventory/view.rs` also publishes the main inventory's saved sort into
the UI context each frame. Texture/material trees inherit that sort directly;
independent inventory selectors share an immutable order projection, cached by
inventory roots/generation, sort and language. They reuse the inventory's
comparator and descendant-based folder dates, including support-folder pins
and Library placement. Environment catalogues accept item ranks rather than
owning a separate ordering policy. Local previews use natural name order.
`ui/texture_library/windows.rs` owns independent, resizable texture/material
windows keyed by inventory item. `texture_view.rs` handles fitted, ratio-constrained
or native-pixel images and worker decoding/export. Profile photos share its image
canvas and controls, with a worker-decoded preview for the selected Second life or
First life asset, independently of the small UI thumbnail. `material_editor.rs` owns isolated
PBR drafts, map pickers and save state with the importer's map-row presentation and sphere/cube/Aurora
geometry. Its bounded scroll viewport and measured footer keep wrapped settings
from expanding the floater. PBR preview workers also prepare the four map thumbnails
from their cached sources, so an already resident scene map needs no new GPU upload;
shape and factor changes reuse unchanged map pixels and thumbnails.
Inventory and library opens share these windows
and the same bounded model cache; reopening an item raises/restores its window
without resetting its camera. Each window survives closing either source panel,
and closes independently. Texture and material windows each persist their own
last manually resized size in library preferences; only new windows inherit it.
Shared floater resize completion events exclude placement, minimization and
screen fitting, and settings save once on release. Unsaved inventory documents
keep their editor/draft.
`ui/widgets/cascade.rs` ports Firestorm's inventory floater registry placement:
same-type instances follow creation order, skipping hidden/minimized anchors,
with a human-requested 32 px right/down offset (twice Firestorm's default) and fitting to the work area. Raising an existing
item preserves its geometry. Inventory and object script editors share a group;
textures and materials use separate groups and retain their size preferences.
Notecard, script, animation, sound and gesture groups remember their primary
window's geometry. `ui/inventory/document.rs` creates independent document states
inside the inventory's auxiliary window list (nil root, no tree); they retain
their own drafts, save acknowledgements and close confirmations, and opening
the same item raises/restores it. Resource, notecard and script integration
tests exercise the shared placement with their actual floaters.
`ui/texture_library/catalogue.rs` caches the `#Aurora/Aurora Library` subtree
by inventory generation. Textures, materials, objects, animations, sounds,
notecards, landmarks and scripts are classified by actual asset type, in any
real descendant folder. Links and the bridge's sibling data are excluded.
The eight English default folder names are conventions. Missing defaults are
created only after fetching the parent listing; old library folders move under
the unified root without changing item IDs. A checked type menu combines types
with OR, then folder/search/favorites/recent with AND. Non-surface cards open existing native
players, document editors or landmark profiles.
`previews.rs` assembles bounded, revisioned PBR models on workers
and draws them with the importer's shared GPU preview. `app/texture_library.rs`
routes inventory mutations, library clicks on selected edit faces and
permission-aware world face drops through the
existing build and task-inventory operations. Object cards use inventory thumbnails,
falling back to distinct single/coalesced-object icons, and share the inventory
rez drag path (`app/inventory_drag.rs`), including permission checks and no-copy consumption.
Inventory rows, pickers, properties, embedded items, Marketplace and task contents
share a flags-aware icon selector for single/coalesced objects.
`Settings::texture_library`
persists presentation, favorites and 32 deduplicated recent resources per avatar.
Recent use is recorded for application, rez, contents insertion, document opening,
playback and landmark teleport, rather than texture/material preview or selection. Ctrl+Space and the
customizable toolbar command toggle the current floater. `demo_texture_library.rs`
provides actual synthetic inventory folders, tileable textures and PBR materials offline.
Inventory texture previews retain the real item as drag cargo, sharing that
world-drop path with inventory rows and library swatches. A dedicated handle
in each material preview supplies the same cargo without consuming the
3D scene's orbit gesture.

`ui/inventory/permissions.rs` renders the current owner's C/M/T suffix and,
when the owning window's switch is on, appends the next owner's C/M/T after
an arrow. Inventory/folder windows and build Contents keep independent session
flags. Link rows resolve both masks from the original item; unavailable masks
remain unknown. The shared visibility control never changes permissions.

`ui/inventory/rows.rs` lists the plain inventory tree (Inventory tab and
folder windows) as flat rows, rebuilt only when the inventory, the sort or an
open folder changes, and lays out only the rows in sight; it keeps each
window's open folders (the main window's with the UI layout). Searches,
filters and the other tabs use the flattened result tree of
`ui/inventory/view.rs`, drawn the same way.

`ui/inventory/drag.rs` snapshots the full Ctrl/Shift selection at drag start
and moves it through `world/inventory/actions.rs::move_selection`. A separate
batch payload prevents single-item world, attachment and editor targets from
silently taking only the first item; existing single-item/folder cargo stays
compatible. Rows claim drops before the enclosing inventory surface, including
invalid targets. Validation is cached by cargo, destination, inventory generation
and protection/worn state, then repeated on release. `drag_navigation.rs`
toggles folders after a one-second hover, once per visit, independently of drop
validity. It scrolls the actual inventory viewport (including virtualized search
results) near its top/bottom edges and outside it vertically, with elapsed-time
acceleration and ownership of the last entered inventory window. The final drag cursor and selection-count
badge are drawn after all windows. `drag_tests.rs` drives actual Ctrl-selection
and pointer events; `drag_navigation_tests.rs` covers hover latching, scrolling,
hidden sources, filtered lists, window ownership and cancellation.
`drag_demo.rs` provides offline move/hover/rejection/scroll/fold scenarios.

`ui/toolbar.rs` owns Firestorm's three toolbars (the bottom one, left and
right), their context menus and drag/drop edits; a command stands in one of
them at most. `toolbar/catalogue.rs` lists Firestorm's toybox commands and
Aurora's own, with whether Aurora runs them; `toolbar/editor.rs` is the
"Toolbar buttons" window (tiles, each toolbar's mode / alignment / size,
Locked, Clear all, Restore defaults). `Settings::toolbar` stores the three
button lists, each toolbar's style and the lock, shared by all accounts at the
human's request; a style left unset follows the default of the look in use.
The first account entering the world after the update supplies its former
per-account layout; an existing shared layout always wins. Legacy layouts
survive pre-login saves until migration. Without an override the skin's live
bottom layout remains authoritative. Edits save only on release or an
explicit action, and an empty override stays empty. `ui/bars/side.rs` draws
the left and right toolbars (glass islands with the islands, full-height
panels with Firestorm's bars) and the windows' room beside them;
`ui/bars/sizing.rs` lays out a toolbar's buttons by mode and layout style on
one row, cutting labels before buttons narrow.

`world/bridge.rs` owns the Aurora attachment factory and confirmed position-lock state. It creates `#Aurora/Aurora Bridge` under the inventory root, migrates the first version's existing object/strokes into that subfolder, copies the library rock only when needed, attaches it at Center 2, installs the bundled `world/bridge/bridge.lsl` in Mono, and saves the modified object by detach/reattach. Compilation first waits for a hello from the running script; the detach then waits five seconds after the attachment disappears, as FSLSLBridgeReAttachTimer does. No inventory asset UUID change is required; the simulator does not guarantee such a receipt. Missing startup replies query the actual script-running state and start a stopped script once. A ready bridge whose requests get no HTTP reply is watched too: after 10 s the viewer queries the script-running state, and a stopped script (a run-time error such as Stack-Heap Collision) or 90 s of silence reinstalls it through the Repair path, at most three times in ten minutes. The bridge object holds two scripts: the movement controller `bridge.lsl` and `query.lsl`, which serves the read-only radar heights and script/object information with its own HTTP-in URL and token, announced by `<auroraBridgeQuery>` in answer to the same discovery probe. Mono gives each script 64 KB and the controller has little to spare, so a query can no longer halt Movelock; the controller's status reports its lowest free memory for the log. Older bridges get only the missing script. The query service is reinstalled only when the simulator reports it stopped, never by silence alone, and reaching the reinstall limit for it leaves the controller working. A region change keeps an installed bridge: the scripts only renew their HTTP-in URLs, so the viewer waits for the new hello in Handshake (probe and running check) and reinstalls only after 30 s without one. Only a hello that renewed a live endpoint is replayed across a region change, never the one that made the bridge ready in the old region. A controller 404 (the scripts never answer 404 themselves, except query.lsl's "unavailable" for a missing object) means a dead URL and triggers the same wait instead of an error. Discovery uses owner-filtered, object-addressed requests on a negative channel, transported through ScriptDialogReply as in the Legacy Viewer. LSL replies only to a bounded viewer probe, avoiding unsolicited announcement loops; private startup/permission/URL status and localized timeout stages remain separate. The stable object/script name `#Aurora Bridge v1.0` permits in-place upgrades; the handshake version is 1.21. Copy callbacks, task inventory and compilation acknowledgements gate every transition. Script initialization is idempotent across repeated state_entry/attach calls; on_rez clears startup flags without resetting the script, preserving the queued attach event; readiness requires both the URL and the actual TAKE_CONTROLS grant, avoiding stale permission events. A handshake is accepted only from the exact worn object in the current region, with the expected folder/version and a valid simulator HTTPS URL. The URL allowlist includes current secondlife.io simulators as well as the legacy domains, and an optional trailing slash on the base capability. Bridge HTTPS uses the same public/Linden CA trust and simulator-hostname policy as the capability client, with certificate-chain verification and no redirects. Response rejection reasons (format/version/folder/endpoint/token) remain separate from script-startup and permission status; diagnostic logs never contain private payloads. Endpoint credentials are transient and redacted. `aurora-net/src/bridge.rs` correlates authenticated background requests. Motion updates coalesce; urgent release/lock changes supersede delayed requests with a higher sequence. Movelock activation is latched in LSL until explicitly disabled; it has no viewer keepalive or lease. A 10 Hz timer maintains the original anchor autonomously, including across simulator URL renewal. Only LSL control events release/reanchor it; viewer motion hints, physical drift and network replies do not decide the hold. The toolbar, menu, switch and indicator follow the saved user selection; provisioning/errors remain separate diagnostics. Region/sitting/movement changes do not change that selection; legacy automatic region-off settings are ignored. Movelock always passes native movement through. Simulator control events release the scripted hold immediately and capture a new anchor on release; neither keyboard masks nor the obsolete hard-lock preference can block movement. No dedicated Movelock toggle sound is emitted. Sitting pauses; region changes release the anchor. Explicit inventory/outfit detachment releases Movelock and pauses bridge recovery for the current session, including across regions, until the user enables Movelock, wears the bridge or chooses Repair; the factory save is exempt. Unexpected attachment loss still uses normal recovery. `Settings::bridge`, `app/bridge.rs` and `ui/movelock.rs` provide per-avatar settings, Ctrl+Alt+P, toolbar/menu and status. The offline demo exercises lifecycle only, without running simulator physics.

`movement.rs` owns waterline hysteresis, surface/diving states, motor settings and steep-obstacle exit rules. `app/movement.rs` samples Linden water/terrain and reachable obstacle clearance at most five times a second. Native swim up/down flags are held back until the matching swimming mode is acknowledged, then delivered to the script control capture so a voluntary dive can release Movelock without an HTTP input relay. Fast-up stays suppressed. Capture forwarding expires one second after the acknowledged command was sent, before the script motor watchdog; delayed replies and control-status chat cannot extend it. Short exit climbs do not permit vertical forwarding. The script keeps capture armed while Movelock holds and suspends only its motion motor, preventing two motors from adding acceleration. Motor scheduling is adapted from the maintainer-supplied legacy movement scripts: the viewer sends mode, speed and avatar height; LSL derives normalized direction from current simulator controls and camera rotation (llGetCameraRot with PERMISSION_TRACK_CAMERA requested together with TAKE_CONTROLS), as the supplied controller does. Run/swim/ordinary flight retain only horizontal camera heading; mouselook flight also retains camera pitch. Missing tracking permission falls back to owner rotation without blocking Movelock readiness. A near-vertical camera falls back to avatar heading for horizontal steering; native shore/passage probes use the same policy. The ordinary AgentUpdate camera axes already feed the simulator; no HTTP orientation relay or camera-control permission is needed. Run/flight keep native controls. Running stops MoveToTarget and uses a bounded horizontal continuous-force regulator: llSetForce with llGetMassMKS, the continuous-force mechanism and avatar force scale used in Firestorm bridge control() for flight assist. Protocol 1.12 restores this authority after the v1.11 legacy-mass substitution also reduced the force cap and slew by 100. The current script controls/camera/velocity drive it locally; no delayed viewer thrust vectors are used. A proportional term plus bounded compensation for native braking maintains the requested speed, preserving that compensation at the target to avoid a repeated speed sawtooth. Stalls and sharp turns reset compensation, and faster forward motion unwinds it; no reverse braking force is applied. The run controller ramps the target up by at most 4 m/s per second and upward thrust by 20 MKS mass-normalized units per second, caps that request at 400 units and requests a 50 ms timer while running. The delivered-force limit bounds integral growth; lower targets, overspeed and release can cut thrust immediately. A distinct ramping state avoids reporting the final target as reached during acceleration. These are controller bounds, not a claim about actual Havok acceleration. Native collisions and gravity remain in charge; there is no altitude target or look-ahead ray veto. Key release, vertical controls, airborne/marked vertical motion and Movelock clear the persistent force before tick throttling; mode switch, sitting, watchdog, region change, detach and release clear it too. The feedback introduced in protocol 1.10 appends bounded motor mode/run state, measured horizontal speed/target in cm/s and a cumulative thrust count to private acknowledgements. The native options show recent actual script feedback separately from user activation; it expires conservatively after two seconds without a matched HTTP acknowledgement. Protocol 1.12 additionally reports bounded requested force per MKS mass, the gradual target and a cumulative controller-reset count; the options expose requested thrust as a percentage of its limit and report saturation separately. Normal releases, sharp turns and safety stops all count as resets; none changes user activation. These fields diagnose authority/reset problems and do not measure physical acceleration. No credentials or identifiers enter that feedback. Flight retains short MoveToTarget goals with a 0.2-second timescale and one idle hover anchor. Swimming captures native translation, does not enable native flight, bounds/decays entry inertia, keeps a height-aware surface target or an anchored underwater depth, and caps targets below water. Probes/steering are throttled; zero-length idle corrections keep their anchor without a ray. A 1.5-second watchdog releases stale enhanced movement; Movelock itself remains autonomous. Motor payloads carry settings and one-shot passage ids, never movement hints for Movelock. A stalled, settled surface swimmer can climb briefly (350 ms, at most 2.5 m/s), then restores its previous flight state; underwater walls and tall obstructions do not cause automatic exits. Running, flying and swimming have independent persisted switches, all off initially, with no global activation. The toolbar active state is their logical OR even when the options are closed or bridge provisioning is pending. Migration consumes the old global flag: globally disabled accounts keep all features off, while enabled accounts retain individual choices. Running uses a factor from x1 to x5, default x1: native running receives no target at x1 and walking is unchanged. Higher factors multiply the documented 5.13 m/s level-terrain run speed, never measured velocity, and a faster forward speed unwinds the run compensation without a reverse braking force. Legacy default run speeds become x1; custom boosts migrate to bounded factors. Measured simulator speed remains visible. A surface flotation offset from -0.50 to +0.50 m (1 cm steps, default zero) adjusts the surface target, dive hysteresis and underwater ceiling. Disabling swimming immediately cancels any exit pulse, stops swim animations and releases the swim motor. Mode 1 carries its run factor in parameter 0; modes 3/6 carry the flotation offset in parameter 2, while mode 4 retains its bounded climb speed. Flight uses a factor from x1 to x5, default x1, against native directional speeds (16 m/s, 22.87 m/s downward). Former 10/25 m/s defaults become x1; custom boosts migrate to bounded factors. `movement/strokes.rs` preloads six maintainer-contributed QAvimator BVHs from `assets/animations/swimming/` on a worker, strips root translations and closes loops. These clips remain available for explicit local/offline previews; normal swimming requires the selected AO inventory assets, and their private preview IDs never enter network animation commands. The former bundled-pack uploader and automatic discovery/seeding are removed. Swimming uses explicit per-set AO inventory links and native `AgentAnimation` so other viewers see the selected assets; no private bundled fallback is broadcast. `movement/run.rs` supplies a bounded enhanced-run pose guard using the avatar contact plane (including elevated platforms), raw airborne/jump signals, vertical velocity and recent support: at most 250 ms / 0.35 m of contact loss, with immediate release for deliberate jumps, marked falls, suspension, sitting, flight or disabled enhancement. `ao/runtime.rs` selects the configured Running state with a continuous signal during that grace and preserves transient landing completion. Without an available Aurora AO running clip, it owns/broadcasts the standard run through AgentAnimation, reasserting on a new fall signal or a missing owned animation at a bounded rate handing it back to ordinary running without flicker and clearing stock-run ownership on release so later native runs are never hidden as stale echoes. The authoritative raw signals and unrelated foreign animations remain intact; scripted third-party AO replacements can still outrank the stock fallback. No LSL animation override or extra permission is installed. Swimming defaults to 5 m/s; migration updates only the previous 2 m/s default and preserves custom speeds. Ctrl plus the configured jump key (PageUp or E by default) uses conservative avatar-sized bounds to select a free target beyond a nearby obstacle, then submits a physical passage displacement of at most 8 m. There is no teleport or client position overwrite. The viewer allows 1.5 s for delivery/completion; the LSL pulse stops at its target or after 1.2 s. A per-passage action id prevents repeated POSTs from advancing/restarting a completed goal. Sitting, suspension and Movelock cancel/prevent a pulse. It bypasses the normal boost ray, as the supplied cross-wall option did; impassable simulator collisions can still stop it.


`scene/auto_hover.rs` estimates ground contact after body attachments change. It snapshots the shaped rest skeleton and attachment geometry, decodes high-LOD meshes and blends only vertices selected by nonzero foot weights on the background pool. HUDs without avatar foot skin are excluded; stale estimates are discarded after further outfit changes. The Hover Height floater exposes the saved automatic option (off by default; existing saved choices are preserved), using the existing per-account height persistence and AgentPreferences write queue.
`demo_auto_hover.rs` supplies rigid foot linksets and a rigged shoe asset for the `AURORA_DEMO_HOVER_HEIGHT=auto-*` scenarios.
The scene compares a lightweight signature every frame; it does not continuously decode or scan vertices. Body-attachment/asset changes and the shaped rest skeleton invalidate the cached estimate after 200 ms of settling. Flight, seating, ordinary motion, animation, streamed physical size and contact-plane changes do not. Calibration waits for a known mesh's geometry and declared skin metadata, retaining the last height for unavailable assets. The hover floater separates manual, automatic and common controls through shared Aurora dividers; automatic Lower/Reset/Raise changes only the saved offset, keeps auto active and immediately applies the cached estimate.
`AURORA_TEST_NEUTRAL_POSE=1` adds a process-only Hover Height diagnostic, initially off so test launches keep normal animations. When enabled manually, `Scene::update_poses` still evaluates live motions but replaces only our rendered palette with auto hover's shaped neutral skeleton. Attachment anchors use that same palette; switching off restores the current animated pose without changing server signals.
Avatar full/terse updates retain the simulator foot plane in `world/objects.rs` for movement. Automatic hover does not read it: the human-requested calibration uses the anatomical standing reference in an empty world, independent of any terrain, platform, world position, contact availability or movement state. The saved `auto_hover_offset` is added to each raw estimate before the final hover limits; its slider reuses the cached estimate for live previews and commits the height/settings on release. The floater shows height controls, automatic adjustment, its fixed offset and camera preference, with the optional neutral-pose test switch.
Calibration reconstructs the standing physical box in an empty world. Its SL height is `clamp(shape_body_height + shape_hover, 1.1, 2.45) + 0.2`: the shape body height precedes mesh overrides, and the extra 0.2 m is the standing physical bounding-box padding documented by [llGetAgentSize](https://wiki.secondlife.com/wiki/LlGetAgentSize). OpenSim uses the shape body height without SL hover/padding, matching the Legacy Viewer's AgentSetAppearance branch. `AvatarSkeleton` retains shape body height/hover and `neutral_root_height`, the actual neutral pelvis minus the standing rendered root correction (`hover + pelvis_fixup - height/2 + pelvis_to_foot`). Subtracting the physical half-height gives the box bottom in neutral skeleton space. This reference follows arbitrary mesh pelvis translations just like the measured vertices; it never reads seated/standing streamed size, contact or the current animated palette.
Foot-influenced rigged vertices count at every attachment point; the regression iterates all points declared by the library, including HUD points. An ordinary HUD without avatar foot skin cannot invalidate calibration. Rigid geometry counts under the knee/ankle/foot chains, including lower-leg attachment offsets and linked children. If no rigged foot geometry replaces the system feet, their morphed and skinned lower-body vertices also enter the minimum. The raw hover is physical-box bottom minus the lowest selected neutral vertex. Numeric `auto hover standing model`, `auto hover estimate`, `auto hover rigid surface` and `auto hover height` logs expose the box, root reference, lowest point, rigid size/transform, raw result, offset and bounded height without identifiers. Decoding, system morphing, skinning and vertex scans remain background jobs.

`settings/hover_height.rs` binds automatic hover ON/OFF, its offset, camera
following and foot grounding options to the logged-in avatar UUID in
`Settings::hover_height_accounts`. Login restores the account before world
updates; every settings save snapshots the active account. Former shared
options migrate once to the first account, including across pre-login saves;
other new accounts use defaults. Reset keeps other accounts' options.
Standing and both sitting heights remain in each account's `hover_height.json`.
The account's Hover Height window remains accessible while region support is
unavailable, so its automatic and camera options are still editable. Manual
height editing follows the region's feature; only server writes additionally
wait for AgentPreferences, as in LLFloaterHoverHeight::updateEditEnabled and
LLVOAvatarSelf::sendHoverHeight. This is a
deliberate UI difference from the Legacy Viewer's disabled menu entry.
`aurora-net/src/caps.rs::simulator_features` retries failed, timed-out or
malformed feature GETs with bounded backoff, following
LLViewerRegionImpl::requestSimulatorFeatureCoro's 30-attempt limit. Each
simulator owns its request task and aborts it on seed replacement or logout.

`ui/hover_height.rs` exposes the avatar's -3..3 m hover adjustment and the
account's camera preference. `world/hover_height.rs` owns local preview, the
committed value, ordered background persistence in the account's
`hover_height.json`, and one outstanding server write at a time. The app
resends after region changes only when SimulatorFeatures and AgentPreferences
allow it. `NetCommand::SetHoverHeight` posts one finite `hover_height` real;
its correlated result supports retry without letting an old reply overwrite
a newer edit. The skeleton caches only the shape offset; scene transforms
add the current hover height independently of pose updates. Camera roots
start from the unshifted object position and add hover only when enabled,
avoiding a one-frame mismatch and floating-point cancellation while dragging.
`app/hover_height.rs` drives the lifecycle and the offline scenario.
`world/flying_at_exit.rs` keeps, in the account's `flying_at_exit.json`,
whether the last session ended flying; the app writes it at logout and sets
the agent flying from it at login.

Avatar name tag height is evaluated with each avatar's skeleton in
`scene/anim.rs`: shape and mesh joint offsets, plus the upper translation
envelope of each animation track (cached by `BoundAnim` when loaded).
The neutral upright height ignores animated rotations and sample-to-sample
bobbing. `Scene::name_tag_anchor` keeps it above the root with 0.2 m clearance;
both drawing (`ui/hud.rs`) and click depth (`App::pick_name_tag`) use that anchor.
This deliberately differs from Firestorm's posed-head tracking, at the
human's request (#261). An animation change may change the height envelope.

Gestures: `aurora-assets/src/gesture.rs` reads/writes the bounded Firestorm v2 newline format. `gestures.rs` owns active definitions and a nonblocking playback state machine; `gestures/keys.rs` and `windows.rs` map SL key/mask and Windows virtual-key values and `builtins.rs` lists Firestorm's 69 stock animations. `app/gestures.rs` correlates loads/saves, restores login/COF gestures, preloads motion/sound assets and routes effects to the existing avatar/audio/chat systems. `ui/gestures.rs` and `ui/gestures/editor.rs` provide the Ctrl+G manager and independent draft editors through shared Aurora floaters. `ui/chat/completion.rs` selects inline trigger suffixes in both local-chat inputs, using the active manager's case-insensitive exact/common-prefix matching. Inventory links resolve to original items; activation labels are mirrored in `World::active_gestures`. Per-account enable/filter preferences live in settings; active gestures persist through the server. `aurora-net/src/session/gestures.rs` loads and parses on network/workers and sends the Firestorm activation/sound messages; uploads use UpdateGestureAgentInventory and inventory read-back.

`shape_editor/` owns the worn shape draft. `catalog.rs` lists the 82 tweakable
shape controls from the shipped `avatar_lad.xml`, including edit groups, order,
sex filters, ranges and hint cameras; its test checks the catalogue against the
XML. `aurora-assets/wearable.rs` parses the bounded LLWearable format and retains
permissions, sale, textures and unknown sections when parameters are changed.
`ui/shape.rs` uses Aurora's Floater, tabs, surfaces and modals, with visible-row
virtualisation and a bottom-anchored Save As / Cancel / Save footer. The footer
measures pending/status labels before the list receives the remaining height,
so asynchronous saves and wrapped errors never enlarge the floater. `app/shape_editor.rs`
opens it from Appearance, the self-avatar/attachment menu or the worn shape's
inventory menu, restores the camera
on closing and provides `AURORA_DEMO_SHAPE`.
`shape_editor/camera.rs` frames the posed skeleton by category (whole body,
head, torso or legs), using the existing camera transition.

The draft overrides only our avatar's visual parameter view in `World`; incoming
appearances stay authoritative underneath and Cancel restores the newest one.
`shape_editor/preview.rs` shares compressed texture/mesh sources and the frozen
local pose, decodes textures and mesh geometry on workers, applies the
shape and joint overrides independently for visible minimum/maximum hints, then
uses the ordinary `aurora-render/preview.rs` GPU pass without a readback. At most
one snapshot and one hint batch are pending; stale replies are discarded and
offscreen hint models are released. BOM visibility shares the world's system-part
mask, with all bake slots mapped for the mesh faces. Static meshes follow their
attachment joint and local parent chain. Content signatures exclude animation
cycles and sort attachment identities; active snapshot/model keys remain separate
from pending ones, so refreshes replace ready images without a loading flash.
Decoded geometry is shared across texture updates; temporarily missing source
bytes retain the same asset's ready geometry/pixels.
Texture sources include progressive JPEG2000, cache-only BC7 and procedural
demo pixels, with small decodes on workers and no frame-thread disk access.
Reserved blank/transparent textures carry the world's exact one-pixel values
without requiring a cached asset. Progressive JPEG2000 hints respect the level
already decoded by the streamer. The preview pass draws opaque/masked parts
before blended layers, whose pipeline does not write depth over the skin.
`shape_editor/material.rs` resolves texture entries, legacy normal/alpha settings,
PBR materials and face overrides, with BOM redirection and per-map UV transforms.
Dragging updates the live avatar immediately; hints coalesce after release and
180 ms without edits, and obsolete workers stop before the next comparison.
Each slider's own endpoints ignore its current value. Immutable rigged vertices
and indices are shared across models; the render preview cache also shares their
GPU buffers and textures, releasing unused assets through weak owners.
`shape_editor/demo.rs` builds rigged/static mesh-head fixtures on workers
through the ordinary asset codec, including PBR and continuous-slider scenarios.
The category row consumes discrete wheel events once per frame and accumulates
trackpad deltas. Hint visibility and height units are saved
in Settings. The main scene continues to use its normal shape/morph streaming.

`aurora-net/session/wearable_save.rs` checks shape ownership and Modify/Copy,
uploads the wearable through the existing AssetUpload/Xfer path, preserves item
permissions for Save As through CopyInventoryItem, patches only the transaction
`hash_id` through AIS and reads the resulting asset back before acknowledging
success. Non-modifiable shapes can be copied only unchanged. Requests and replies
are scoped by request/item/asset identities; failed saves retain the draft. A
confirmed save uses the existing outfit/COF path to request server appearance.
The same path saves a physics layer (clothing, wearable type 15).

`physics_editor.rs` owns the worn physics layer's draft: the 33 tweakable
physics parameters of `avatar_lad.xml` in the seven tabs of the Legacy Viewer's
panel (breasts bounce / cleavage / sway, belly bounce, butt bounce / sway,
advanced), hidden by sex as there, checked against the XML by a test. Its
weights are previewed through the same `World` visual-parameter view as the
shape draft, so `scene/body_physics.rs` bounces the body with them at once.
"New physics" (outfit gallery) creates a default layer in the Clothing folder,
wears it and opens the editor once the outfit wears it. `ui/physics.rs` is
its floater (single-expansion accordion, 0–100 sliders, the shape editor's
footer and dialogs); `app/physics_editor.rs` opens it from the worn layer's
menus, keeps one appearance editor open at a time and provides
`AURORA_DEMO_PHYSICS_EDIT`.

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
`assets/lsl/keywords.json` (signatures and help in `assets/lsl/reference.json`);
`lsl_keywords.rs` holds the catalogue in use, the embedded one or the one the
agent's region names (SimulatorFeatures' LSLSyntaxId, followed by
`world/lsl_syntax.rs`, fetched and cached by `aurora-net/src/lsl_syntax.rs`).
The lexical analysis and the formats stay cached per document as long as the
source, the theme, the font and that catalogue do not change; the
source sent to the simulator keeps its bytes exactly when the preprocessor is
disabled.

`ui/scripts/view.rs` composes the menus, the toolbar, the line gutter and the
clickable diagnostics. `editing.rs` shares the native cursor and history
between the commands and typing; `dialogs.rs` carries search/replace, the
preferences and the keyword help.
`Editors` owns one search/replace panel shared by all scripts, following
`LLFloaterSearchReplace::show`. Opening search rebinds its editor, retaining
the query, replacement and options. It is drawn once after all editors have
handled commands. The compact floating panel tries the owner's right side,
then its left (reversed by the saved left-side preference), within the work
area; minimizing or closing its owner hides it. If neither side fits, its
top follows the measured body start, preserving the title controls, and a
dependent sublayer keeps it directly above its owner. Unrelated windows can
still cover both. A second saved preference embeds the same controls in the
header's right column when the measured toolbar and search both fit. Its
two compact rows share the normal menu/toolbar height, without a close
button; the text Find / Replace / Replace all buttons share the second row,
while status feedback truncates beside the options with full hover text.
The width accounts for translated option labels. The
header reserves a child UI; the shared panel renders it after all commands,
preserving field IDs, focus and options across changes of width or owner.
Script floaters opt into the shared partial-outside policy even with edge
stops enabled: dragging can cross the sides/bottom while retaining the
standard reachable title grip. Other floaters keep the global edge policy.
Opening an editor focuses character zero once its source is ready; only the
latest explicit opening may take focus after an asynchronous load.
`ui/scripts/focus.rs` tracks activation from clicks anywhere in the editor,
its search and its popup menus, independently of text-field focus. Ctrl+F
is dispatched once after editor commands and uses that active document;
clicking another window or the world releases it. `ui/find_shortcut.rs`
uses the same activation before allowing the viewer's global Search action.
An explicit search focus request selects the whole query in either layout;
ordinary frames, layout transitions and Enter keep the query's current caret.
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
| `imports/`, `app/imports.rs` | Per-file import drafts removed after acknowledged uploads (discarded on a worker), local previews and unfinished drafts kept; preview, selection of the material maps, texture / animation / LOD / physics parameters; sequential upload queue with deduplicated material dependencies, mesh quote frozen before confirmation and processing of the files on the workers. `local.rs` watches the local textures, materials and meshes and prepares their GPU resources; meshes, textures and materials target the selection or a support prim through visual overlays kept in `world/objects.rs`, without changing the server data. The textures use the scene's staging pool; the render loop installs and frees their GPU slots explicitly. The local picker recognises the three types, including glTF/GLB documents according to their content. `persistence.rs` saves the references/settings and the applications per account/grid in the private profile; read back on a worker, atomic writes batched and flushed on close, missing sources kept. Connection, missing quote, unknown price and account-specific mesh permission denial have distinct statuses. `ui.rs` shows the same Name / Description / Destination rows above every import panel, including the mesh sections; `ui/destination.rs` filters the allowed folders, with a cache and a virtualised list; `ui/material_panel.rs` keeps the 3D canvas and gesture hints stable while loading, beside four distinct PBR cards in a 2 × 2 grid without scrolling, with left-aligned titles and names, right-aligned actions and common parameter rows; `ui/mesh_panel.rs` places metadata beside the preview and its vertical tools, with five tabs and a bounded section viewport above the fixed model/fee summary; `ui/mesh_panel/panels.rs` supplies the Avatar joint/override controls and actual Journal diagnostics; `ui/zoom.rs` enlarges the 3D view with its camera or a square texture preview prepared on a worker at native resolution (2,048 pixels per side maximum, one large GPU cache at a time); `permissions.rs` stores per-type next-owner defaults with migration from the legacy shared preferences, frozen into each confirmed upload step; `ui/permission_panel.rs` exposes only Modify / Copy / Transfer for the five importable types in the main Permissions tab; `animation.rs` keeps an independent clock; `preview.rs` also prepares the imported hand poses, expressions and joint positions |
| `aurora-assets/src/import_*` | Pure validation and conversion: dimensions / alpha / JPEG2000, WAV to Vorbis, BVH to animation, RGB / ORM PBR maps, shared glTF / GLB resources, Collada / glTF geometry, LOD through meshoptimizer, spatial transfer of the weights, convex hulls and serialisation of the mesh into compressed LLSD sections. `import_mesh/vhacd.rs` and `native/vhacd/` wrap V-HACD 4.1: bounded allocations, contained exceptions, atomic cancellation / progress; each model runs on a Rust worker |
| `aurora-net/src/inventory/{upload,mesh_upload}.rs`, `session/asset_imports.rs` | Two-step import protocol, expected cost, read-back of the created item; materials created then saved through their dedicated capability. The mesh transfer URLs stay private to the session, tied to a local token consumed only once and expiring after five minutes |
| `aurora-render/src/preview.rs`, `shaders/preview.wgsl` | 3D previews in the windows: PBR rendering and skinning on the GPU thread, off-screen targets linked directly to egui, deduplication of the shared maps, no GPU readback and no wait on the GPU in the interface loop. Shared orbit, screen-space pan, zoom and camera reset; material previews use Sphere / Cube / Aurora 3D and four worker-prepared thumbnails, with a self-contained bundled GLB |
| `build/replace.rs`, `build/replace/`, `build/ui/replace.rs` | Guided replacement of a linked part: locked target, selection of the replacement, options and progress; steps confirmed by the server, copy of the contents through the resident's inventory, script states preserved, recovery of the initial configuration and offline simulator |
| `ui/inventory/document.rs`, `aurora-net/src/inventory/notecard.rs` | Editing cycle of the documents: draft and save correlated with the window, closing after confirmation, refusal that keeps the text; import of v1 / v2 notecards, v2 export with a bounded UTF-8 length and the permissions of the attachments still referenced preserved |
| `ui/inventory/aurora.rs`, `ui/inventory/aurora/` | Aurora's inventory window (`Settings::classic_inventory` off, Preferences › Aurora › Inventory): one window that widens. `aurora.rs` picks the layout from the body width (narrow below 700 px: icon tabs, search row, the tree, the selection's card; medium: a column of shortcuts and folders, an explorer of one folder with type and date columns, the card; wide from 1020 px: breadcrumb, Ctrl+F field, filter chips, details pane, status bar), counts the shortcuts per inventory generation and runs the explorer on its own `PlainTree` (folders never open in place, a double-click or held cargo goes in); `rows.rs` draws the 28 px rows (pills, star, rights, columns, Wear / Take off and More on hover) over the shared `item_response` / `folder_response`; `rights.rs` the C/M/T letters and the CMT menu (`InventoryPreferences::rights`: mine, next owner, both, none); `sidebar.rs` the shortcuts and the folders-only tree; `toolbar.rs` the bars, the Sort and New menus, the chips; a folder window is the same window under `InventoryUi::scope` (no tabs, its folder as the root of the lists, the column and the breadcrumb); `details.rs` the card and the pane (main action of the double-click, Give, next owner switches patched like the properties) |
| `ui/inventory/embedded.rs` | Items embedded in notecards: icon and name in the text, preview / opening and copy to the inventory, drop at the pointer position; one character per item to preserve selection, deletion and undo |
| `aurora-net/src/session/inventory_preview.rs` | Reading of the notecards and scripts protected by the simulator's inventory transfer (identifiers of the agent, the session, the owner, the object, the item and the asset); bounded reassembly of out-of-order packets, correlation with the simulator and expiry |
| `app/task_notecards.rs`, `aurora-net/src/session/task_notecard.rs` | Notecards in the contents of objects: double-click / Open, shared editor with a state specific to the object and to the item; rights of the object and of the notecard, asset transfer authorised by the inventory, save through UpdateNotecardTaskInventory, read-back and draft kept on refusal |
| `main.rs`, `app.rs`, `app/` | Startup, event loop, one frame of the viewer, interface actions; chat bar commands (`app/chat_commands.rs`), right-click menus (`app/context_menu.rs`: opening, rights on the object, chosen actions), the left button's clicks for scripts (`app/script_clicks.rs`: mouselook bits when a script took the button, touch at the crosshair otherwise, LBUTTON bits for a press nothing takes; the counts of `aurora-net/src/script_controls.rs`), clicks on the world without waiting for the GPU (`app/click_pick.rs`: the depth under the click is requested from the renderer, the click is handled when the reply arrives with the cursor and the keys it had, and what follows it — release, other buttons, mouse movement that moves the camera — waits behind it in order; check `AURORA_CLICK_PICK`, demo clicks `AURORA_DEMO_CLICKS`), HUD move gesture with a configurable key (Ctrl by default) and left click, and offline scenario (`app/hud_drag.rs`), the system cursor hidden and locked by mouselook and the camera drags, and where it comes back (`app/cursor_hold.rs`), the name plate under the pointer (expanded on hover) and the action of a click on it (`app/plate_click.rs`), "Reset skeleton (and animations)" of an avatar and "Stop animations" of ours, with the ResetSkeleton ViewerEffect sent and followed (`app/reset_skeleton.rs`), Me › Avatar health's "Undeform avatar" and "Refresh attachments", and the customise animation while our appearance is edited (`app/avatar_health.rs`), "Derender" of the menus, the Asset blacklist window and `AURORA_DEMO_DERENDER` (`app/derender.rs`), the Dev Toolbox's tools — toggles, copies, live views, windows — and "Send a diagnostic" (header, system configuration, hidden names; Copy, Save as .txt, Log) (`app/dev_toolbox.rs`), its "Debug to log / clipboard" (J1, J2), which ask the names of the worn items they detail, then hand it the lines of `scene/describe.rs` (`app/describe.rs`, `AURORA_DEMO_DESCRIBE`), the double click on the ground (walk, run or teleport there) and the walk frame after frame (`app/double_click.rs`), the object menus' entries that take more than one message — Return, Drop, Put on after a walk to the object, the script actions on a selection, Block particle owner, our attachments' favorites, Link and Take multiple on a kept selection, Touch with the right click's surface (`app/object_menu.rs`; `AURORA_DEMO_OBJECT_MENU` in `app/object_menu_demo.rs`, its objects in `demo_object_menu.rs`) |
| `cli.rs` | Command-line arguments (`--title`, `--fps-limit`, `--language`, `--grid` / `--loginuri`, `--login`, the startup SLURL, `--version`); title with the common version and the build profile |
| `external_url.rs`, `slurl_handoff.rs`, `url_scheme.rs`, `app/external_links.rs` | SLURLs from outside the viewer: what a link from the command line or a web page asks for and what is refused from outside (`external_url.rs`), the named pipe that hands a link to the viewer already running (`slurl_handoff.rs`), the user's `secondlife:` link handler in the Windows registry (`url_scheme.rs`, Preferences › Interface), and their use: login start location, place details, commands once in the world, `--login` (`app/external_links.rs`) |
| `cmdline/` | Chat bar as a command line (FSCmdLine): parsing of the commands, calculator (LLCalc), dice; executed by `app/chat_commands.rs` |
| `agent.rs` | Our avatar (AgentUpdate, extrapolation from the server velocity / acceleration, smoothing like LLDrawable; with Aurora's movement, the lead of `own_motion.rs` added and the corrections riding on the extrapolation). Before movement assistance and control submission, `World::auto_land` clears FLY when descend is held within 0.25 m of terrain or the simulator's object contact plane and horizontal speed is below 4 m/s. Contact uses the reported physical centre/height, independently of visual hover and prediction; no STOP is sent. |
| `own_motion.rs`, `app/own_motion.rs` | Aurora's movement of our avatar (Preferences › Aurora › Movement, Firestorm's by default): the intent of the control flags sent, the lead of the drawn avatar over the simulator's (the keys' motion over the last round trip, given up when the simulator does not follow), the speeds, ramps and delay learned from the simulator's reports, the slow fade of corrections of 0.5 to 3 m, and the walk / run / fly animations started and stopped with the keys over the simulator's signals (`Locomotion`, read by the AO and the pose through `World::own_signals`); never seated. `app/own_motion.rs` runs it each frame, drives the demo's simulated link and answers a sit on any prim of the demo and the stand up from it (`demo::sit_on_prim`, `demo::stand_from_prim`) |
| `autopilot.rs` | Walking our avatar to a point by itself (LLAgent::autoPilot: body turn and walk flags of AgentUpdate), the double click on the world and its setting, where a teleport by double click goes |
| `world/motion.rs` | The other avatars and the objects between two reports of their simulator: extrapolation along the clock of their region (`ObjectStore::motion_time`, advanced by `World::advance_motion` at the region's time dilation, stopped with a silent circuit), and the fading of the gap a report opens with where the object was drawn (`Correction`; while it fades the object counts as moving for the scene, `ObjectStore::expire_corrections` ends it); a report moves from the moment the network read it (`ObjectStore::report_clock`, `World::apply_received`) |
| `ao/`, `app/ao.rs`, `ui/ao.rs` | Built-in Animation Overrider: catalogue of the authoritative `#Aurora/Aurora AO` sets, inventory links and ZHAO import (`inventory.rs`), one startup reconstruction from `#Firestorm/#AO` when the Aurora folder is absent (`bootstrap.rs`), explicit single-set Legacy Viewer copying (`migration.rs`, on-demand picker in `ui/ao/migration.rs`), states / cycles / groups / tracks and remapping of the pose of our avatar only (`runtime.rs`), synthetic assets and transitions (`demo.rs`); AIS operations and asynchronous replies in `app/ao.rs`, management window opened by the AO button of the bottom bar. The raw signals of the simulator are still used for movement and transitions; the overrides are announced through AgentAnimation. Activation and last set saved per account UUID; pause and standing poses specific to the session. |
| `camera/` | The camera like LLAgentCamera: view behind the avatar, Alt camera on a point or an object (`focus.rs`), seat camera, scripted camera (`follow.rs`: LLFollowCam parameters, per-object activation stack held by World and avatar-relative simulation; `world/mod.rs` deactivates seat/attachment linkset sources before parent changes, retaining parameters and unrelated sources), mouselook and its right-button zoom (`mouselook_zoom.rs`: LLToolCompGun's held zoom, the wheel steps, the view limits), transitions, smoothing, pull-back in flight, collision sent by the simulator; the photo window's free camera (`free.rs`: its own position and heading while the avatar stays, moves in the camera's frame like the Legacy Viewer's 3D mouse flycam, optional collisions that slide along objects and the ground; `Camera::update` shows it and keeps the avatar's view behind it); Legacy Viewer settings (`settings.rs`), demo scenarios (`demo.rs`) |
| `world/` | World state received from the network: the favourites (`favorites.rs`: the Favorites folder's landmarks in the account's order, LLFavoritesOrderStorage), objects, regions, terrain, objects of the out-of-range regions set aside until these close or come back into range (`parked.rs`), environment (EEP: cycles and settings assets in `eep.rs`, choice local > parcel > region > default day, local environment and fades in `eep_env.rs`, lists of the environment picker and loading of the folders in `env_select.rs`), selected parcel of "About Land", its rights and its refresh while the window is open (`land.rs`), social, avatar profiles, inventory, groups (active, pinned; the open profile's members and roles in `group_data.rs`, its parcels and accounts in `group_money.rs`), contact sets and nicknames (`contact_sets.rs`), blocks, derendered objects and residents and the account's asset blacklist (`derender.rs`: their updates dropped, kept to come back at once), messages of the Legacy Viewer's LSL bridge (hidden), body orientation, gaze (LookAt), map, place profiles of "Places" and of the separate windows (`place_details.rs`: region by its name, landmark, RemoteParcelRequest, ParcelInfoRequest), the parcel snapshots of the favourites without a thumbnail (`place_snapshots.rs`), landmark assets and handles of their regions (`landmarks.rs`), dated and saved teleport history (`tphistory.rs`), teleport offers and requests from other residents waiting for their answer (`teleport_invites.rs`), residents' inventory offers as cards in the giver's conversation, answered through their notification (`inventory_offers.rs`), calling cards given and received (`calling_cards.rs`), the content rating chosen during the session and its sync with the server (`maturity.rs`), experiences' details, the permission questions waiting for them and the agent's permission for each (`experiences.rs`), Set Home to Here and the map's home that follows it (`home.rs`), the account settings of Preferences › Privacy the grid keeps (`user_info.rs`)… |
| `scene/` | What is sent to the GPU: geometry and LOD, textures (streaming), avatars (skeleton, animations, silhouette), particles, water, reflection probes, world sounds, impostors, environment settings assets (`settings.rs`: sky, water, day cycle) |
| `ui/` | egui interface: bars (`bars.rs`: the Legacy Viewer's full-width bars and what both looks share; `bars/islands.rs`: Aurora's top bar, glass islands over the world sampling the windows' blur, the system island's items, detailed tooltip and right click; `bars/place.rs`: the place island, what it shows by the room it has, its menu; `bars/dock.rs`: the bottom islands, the chat island with the chat bar's field and the toolbar's buttons in a centred dock, one height for both; `bars/side.rs`: the left and right toolbars; `bars/sizing.rs`: the widths of a toolbar's buttons; `bars/favorites.rs`: the favourites in the place island's menu, the row under the islands or the dock's grid, chosen by "Favourites access"; `bars/demo.rs`: `AURORA_DEMO_BARS`; `interface.rs`: the interface hidden by Alt+Shift+U and the bars in mouselook), windows, options, chat, contacts (friends, groups, contact sets: `contacts.rs`), avatar profiles (`profile.rs`, the Legacy Viewer's tabbed window, and `profile/aurora.rs`, the default window with its header and cards, on the same state), world map (`worldmap/`: the full-frame layout's floating panels in `overlay.rs`, the Legacy Viewer's side panel in `mod.rs`), "Places" (`places.rs`: the Legacy Viewer's tabs of favourites, landmarks, teleport history; `places/aurora.rs`: Aurora's look, narrow with tabs and rows that open in place, widened with a rail of sources and folders and a preview; `places/model.rs`: its places, one per landmark asset, their last visit from the history, the history's places without a landmark) and place profiles (`place_details.rs`, also as separate windows), "Create landmark" (`landmark_create.rs`), "Asset blacklist" (`blacklist.rs`: the derendered objects and residents), "Region / Estate" (`region/`: the window, its roles and "View as" in `mod.rs`, the header and the overview in `overview.rs`, Manage's sections in `manage.rs`, its Interventions and their dialogs (message, teleport home, kick, objects returned, restart, schedule) in `interventions.rs`, the Top scripts / Top colliders window in `top_objects.rs`, Environment, Experiences and Covenant with their pickers and scope question in `estate_wide.rs`, the rules' and terrain's Advanced folds (telehub and spawn points, PBR terrain materials and their transforms, RAW file) in `advanced.rs`, the classic window (the Legacy Viewer's eight tabs over the same sections) in `classic.rs`, their changes waiting for "Apply" in `draft.rs`, the questions and notices in `confirm.rs`, People's four estate lists with their pickers, scope question and CSV files in `people.rs`; data in `world/estate.rs`, what the app feeds it (the selection for the telehub, the terrain's materials) in `app/region_window.rs`, protocol in `aurora-net/src/estate.rs`, `session/estate.rs` and `session/terrain_raw.rs` (the RAW file's Xfer)), "About Land" (`land/`, one file per tab, dialogs; Aurora's layout over them in `aurora.rs`, its overview in `overview.rs`, the settings by intent in `manage.rs`, the access lists and object owners in `people.rs`, Script info in `scripts.rs`), the pieces the redesigned windows share (`sheet.rs`: header on the field's face with its gauge, page tabs with "View as", pills, cards with a title and a link, lines of facts, settings rows with their sentence, amber lock banner and line, "Advanced" folds, a column of sections whose navigation follows the scrolling (a row of icons in a compact window), the bar of changes waiting for "Apply", list heads and resident rows), group profile (`group_profile.rs` and `group_profile/`: header, Home, Members, Notices, Land and L$ (`land.rs`), Experiences (`experiences.rs`) and Administration tabs, or the classic window), resident picker (`avatar_picker.rs`), experience profile with its permission (`experience_profile.rs`), web page in a window (`web_view.rs`), the media filter's question and its list in the preferences (`media_filter.rs`), the Marketplace web site in a window (`marketplace.rs`), texture picker, inventory, environment picker and "Personal lighting" (`environment.rs`), maps, debug overlays; sounds caused by the widgets (`sound_cues.rs`: clicks, refused keys, windows); the progress window of a selection's script actions (`script_queue.rs`); right-click menus: shared look (`menu.rs`: Tabler icon, submenus, checked boxes) and entries like the Legacy Viewer (`context.rs`: world, avatar names, groups) |
| `build/` | Building tools like LLFloaterTools: selection and manipulators (`manip.rs`), selection regressions for linked parts and whole objects (`selection_tests.rs`), cycling through the selection under the pointer with CTRL + wheel (`cycle.rs`: units per linkset or prims of a single object, high-precision wheel), direct moving of HUDs (`hud_drag.rs`: root of the linkset, screen / attachment point conversion, local echo and position MultipleObjectUpdate on release), Move (`grab.rs`) and Align (`align.rs`) tools, terrain (`land.rs`), edits of the selection (`edits.rs`), materials and media (`materials.rs`), object contents (`contents.rs`), the script actions of a selection (`script_queue.rs`: Reset / Run / Stop / Remove scripts, object after object), impact and weights (`costs.rs`), the avatars and animated objects whose animations pause while something they carry is selected (`anim_pause.rs`), demo simulator (`demo_sim.rs`); the window and its tabs in `build/ui/` |
| `interaction.rs`, `cursors.rs`, `ui/object_actions.rs` | Rules of the click actions 0–9, inheritance, permissions, cursors drawn from the Tabler icons (Firestorm's rules), buy / pay windows and contents list; transaction after confirmation |
| `hover_tips.rs`, `ui/hover_tips.rs`, `app/hover_tips.rs` | The world's hover tips (LLToolPie::handleToolTip, LLToolTipMgr): the object, avatar or parcel under the pointer and what makes it the same tip, the rest delay, the press that hides it and the room the pointer may cross to the card (`Timing`), the hovered parcels asked per 4 m cell with HOVERED_PARCEL_SEQ_ID and kept by bitmap (`HoverParcels`), the tip's content and its Legacy text (`build`); drawn as Aurora's bubble, card with its buttons or readout, or the Legacy Viewer's text (`ui/hover_tips.rs`); the frame's requests (ObjectPropertiesFamily, GetObjectCost) and the card's actions as context menu actions (`app/hover_tips.rs`); avatars from `Scene::avatar_on_ray`, the hover search leaving them out; demo `AURORA_DEMO_HOVER_TIPS` (`demo_hover_tips.rs`) |
| `app/object_actions.rs` | Triggering of the actions, held touch, physical move, parcel playback, opening of media and camera framing |
| `ui/controls.rs`, `app/controls.rs` | The Controls window (LLFloaterCamera, LLFloaterMove): camera and movement tabs whose joysticks and buttons are held keys (`App::update_move_input`), the four views, object view, saved camera positions (`camera::CameraShot`), or Firestorm's two windows (`Settings::classic_controls`); while the photo window's free camera flies, its camera tab drives it (`App::photo_free_camera` / `photo_free_move` in `app/snapshot.rs`: the movement keys and the window's buttons move the camera instead of the avatar) |
| `ui/pay.rs`, `app/pay.rs` | Paying a resident, a group or (Aurora look) an object (LLFloaterPay): quick and free amounts, message, confirmation above a threshold, Buy L$ for what is missing, the server's answer; Firestorm's window as the classic look of Preferences › Aurora |
| `ui/money_tracker.rs`, `ui/money_tracker/`, `app/money_tracker.rs` | Money tracker (FSMoneyTracker): the L$ received and paid during the session from MoneyBalanceReply's TransactionInfo, while open or always (Track when closed); its list, totals, selection, CSV copy and undo in `money_tracker.rs`, the Aurora look in `aurora.rs`, the Legacy Viewer's window in `legacy.rs`; what is recorded and when, the actions and the demo in the app bridge |
| `ui/buy_currency.rs`, `app/buy_currency.rs`, `aurora-net/src/currency.rs`, `aurora-net/src/session/currency.rs` | Buying L$, opened from the balance of the top bar: window and estimate / confirmation / purchase cycle (LLFloaterBuyCurrency, LLCurrencyUIManager), choice of the grid's help page, end notification and simulated service of the demo; XML-RPC calls `getCurrencyQuote` / `buyCurrency` (sent only once, parameters never logged) |
| `ui/buy_land.rs`, `app/buy_land.rs`, `world/land_buy.rs`, `aurora-net/src/land_tool.rs` | Buying land (LLFloaterBuyLand): the rules of the purchase (who may buy, price, billable area, L$ missing), the window with its steps and the estate covenant, the selection it follows (About Land's or the land tool's), XML-RPC `preflightBuyLandPrep` / `buyLandPrep` on `landtool.php`, then ParcelBuy / ParcelClaim; simulated service of the demo |
| `ui/marketplace_listings.rs`, `app/marketplace_listings.rs`, `world/marketplace.rs`, `world/marketplace/{validate,demo,tests}.rs`, `aurora-net/src/slm.rs`, `aurora-net/src/session/slm_net.rs` | Me › Marketplace listings (LLFloaterMarketplaceListings): the SLM API of the `DirectDelivery` capability (`slm.rs`, pure: routes, JSON bodies, answers and Firestorm's alerts; sent off the frame loop by `slm_net.rs`, capability URL never logged); the listings store of LLMarketplaceData (`world/marketplace.rs`: records, version folders, folders being updated, stock counts, outbox of requests, consistency with the inventory); the audit and the drop checks of LLMarketplaceValidator on a working copy of the folders whose fixes become one inventory mutation (`validate.rs`); window with four tabs, filter, drops, menus by listing depth, associate dialog and audit (`ui/`), drawn with the inventory's tree pieces (`ui/inventory/tree.rs`: caret, type icons, row label, list frame, filter field, drop outline) and listing badges in the style of the permission badges; bridge to the session, the notifications and the demo's simulated SLM server (`app/`, `demo.rs`) |
| `scene/picking.rs` | World rays against triangles shared with displayed prim / sculpt / mesh geometry; exact hit prim rather than overlapping bounds, IGNORE traversed outside build mode, Touch surface data. Right click and edit selection include transparent surfaces without an action, testing candidates in parallel while keeping displayed-depth occlusion; fully invisible objects retain their CPU geometry in the pick index. Blend presses and hover resolve the material mode per intersected face, independent of texel opacity. |
| `scene/drawn.rs` | Click target from the GPU-posed faces under the pixel (aurora-render/src/drawn_pick.rs): animesh, rigged attachments and worn meshes reached where displayed by left click, right click and build tools. Isolated gesture × object-status rule (reaches), posed Touch surface data (face, position, ST / UV, normal and binormal). Blend stops presses without an action and at zero texel alpha; Mask retains its alpha test in GPU picking. The posed point bounds media and prevents fallbacks to a background action or avatar. Deliberate difference from Firestorm, which reaches rigged meshes only by right click and the Move tool. |
| `scene/shown_bakes.rs` | The bake an avatar draws: a new bake waits behind the last one that had pixels until it has its own (LLVOAvatar::updateBakedTextures, last known good bake), held so it downloads; then `World::refresh_bakes` builds the avatar and what shows its bakes again. A rebake never shows the body grey or as the rez grid |
| `scene/rez.rs` | The rez effect (Preferences › Aurora, Aurora / Legacy): nothing loading is ever grey. A face whose texture has not come (its slot still on the shared grey placeholder) is drawn as a grid in the brand gradient by the shader itself (`rez_still_grey` in object.wgsl), near or far, from its first frame, and fades into its texture as soon as that texture shows its first pixels (`TextureStreamer::take_first_texels`, `Scene::rez_first_texels`): no CPU work per object for it. Events get a whole-object effect from a per-object state machine: a rez in the world (ours by FLAGS_CREATE_SELECTED, others' new near us in a region watched for a while) stays in the grid until loaded, then a line rises from its bottom (six at once at most, near and big enough on the screen); avatars get the grid instead of the loading cloud until complete, then one line with what they wear (shown anyway after 20 s, never a cloud again); an avatar already shown whose look changes is never the grid again: it stays as it is until its new look is complete, then a glint; objects streaming in get a glint, a line that hides nothing (`rez_mode::GLINT`), the biggest on the screen first, on four of the six lines; a line only starts for what is not behind a wall (`Scene::segment_blocked`, a few rays through the pick index); worn items rez and derez on their avatar; what the simulator kills gets the reverse, a derez, its last frame kept drawn for 0.7 s by `collect_garbage`. Every path that first syncs an object tells it (`Scene::after_full_sync`). The records carry its start, mode and bounds (`DrawRecord::rez`), `rez_apply` in object.wgsl draws it inside the existing passes and the faces of a line join the glow pass. `AURORA_REZ_EFFECT`, `AURORA_REZ_STYLE`, demo `AURORA_DEMO_REZ_EFFECT` |
| `scene/list_index.rs` | What the draw lists look for in each object slot every frame (avatar, transparent or glowing faces, drawn object, worn HUD): one byte per slot, written where the GPU's object table is written (after the sync of an object, when its faces are reclassified for alpha, when its slot is freed), so as not to walk the `ObjGpu` of every slot ever used; `AURORA_LIST_INDEX=0|check` |
| `scene/hover.rs` | Cached object and surface point under the cursor: compact sphere index maintained by sync, candidates near the ray or displayed depth, reused while the question and nearby scene stay unchanged, refreshed at least every 100 ms; never a click cache. Foreground Blend triangles block hover without an action and at zero alpha, retaining large-face hierarchies; Mask holes remain pass-through. The cached point bounds media hover. Always-tested objects (avatars and rigged meshes in crowds) have classes computed after arrival / change to exclude irrelevant steps. AURORA_HOVER_CHECK compares the full reference search; AURORA_HOVER_LEGACY compares the index / hierarchy strategy; hover summary provides profiling. |
| `scene/pick_accel.rs` | Triangle bounding-volume hierarchy for a picking face: the same ray distance as exhaustive triangle tests, built by a background job when a large mesh / sculpt face is first queried. Hover and the shared Blend query use ready hierarchies; other click surface tests remain exhaustive. |
| `media/` | Media of prims and parcels (objects with media kept up to date by the change stream of the `ObjectStore`, with no pass over all the objects), OpenID cookie of the grid's web pages (`openid.rs`), private browser profile deleted after use for the Web tab of "Search" (`private_profile.rs`: its own folder under `cache\web-private`, usage lock, deletion when the browser stops, on exit and at the next startup), download of Aurora's media plugins pack when no plugins are found, with retries (`pack.rs`), the media filter (`filter.rs`: the account's allow / deny list of sites, `medialist.xml`, and the questions; checked before a parcel media, a prim media or the parcel music reaches a site) |
| `demo_tex_refresh.rs` | `AURORA_DEMO_TEXREFRESH` scenario: a panel whose texture file in the disk cache is stale, the right one on the local asset server (`demo_asset_server.rs`); "Refresh textures" (or `auto`) downloads it again |
| `demo_blend_shadows.rs` | `AURORA_DEMO_BLEND_SHADOWS` scenario: alpha-blended panels in the sun (greenhouse pane with opaque frame, face and texture alpha above and under the shadow cutoffs), or 400 greenhouse panes for the cost of blended casters |
| `demo_plants.rs` | `AURORA_DEMO_PLANTS` scenario: the 21 tree species and 6 grass species near or far along the start view, or a forest of about 600 trees and 400 grass tufts; stand-in species textures drawn under their ids, in this scenario only |
| `demo_sculpt.rs` | `AURORA_DEMO_SCULPT_ZERO` scenario: ordinary torus and two sculpts sharing its path/profile, subtype zero and sphere, with a synthetic JPEG2000 map in the demo cache |
| `demo_social_menu.rs`, `app/social_menu_demo.rs` | `AURORA_DEMO_SOCIAL_MENU` scenario: a resident's "Invite to group" (group picker, then the invitation), "Add to contact set" (a new set), "Block particles" (the fountain they own) and calling cards (given, accepted by the demo server, or offered by the resident) |
| `demo_notifs.rs` | `AURORA_DEMO_NOTIFS` scenario: the notification center's mockup (an IM, a friend online, a teleport and an inventory offer, a burst of five), in do not disturb, with the list or the history open, or with Firestorm's toasts |
| `demo_seat_kill.rs` | `AURORA_DEMO_SEAT_KILL` scenario (issue #746): a resident's seat deleted before the simulator stands them up; they stay where they sat, with what they wear |
| `demo_presence.rs` | `AURORA_DEMO_PRESENCE` scenario: the friends' online / offline toasts at one of their six places, above the nearby chat lines, waiting at the top right under an IM card and the open list (shown, or dropped after 3.5 s) |
| `demo_attention.rs` | `AURORA_DEMO_ATTENTION` scenario: the window to the taskbar, then an IM, mentions in nearby and group chat, an object's message, a teleport offer and a script dialog |
| `demo_toasts.rs` | `AURORA_DEMO_TOASTS` scenario: a friend offline then online, two IMs from one resident, a group IM, then the bell list or a click on the IM card (window closed or minimized) |
| `demo_chatlog.rs` | `AURORA_DEMO_CHATLOG` scenario: history files of the demo account (in its own folder), conversation with history, search, History window on a long file, IMs received offline, long conversation to measure the view |
| `demo.rs`, `demo_land.rs`, `demo_region.rs`, `demo_place.rs`, `demo_eep.rs`, `demo_env.rs`, `demo_sky.rs`, `demo_stream.rs`, `demo_arrival.rs`, `demo_turn.rs`, `demo_asset_server.rs`, `demo_glyphs.rs`, `demo_tags.rs`, `demo_deform.rs`, `demo_link.rs`, `demo_tangents.rs`, `demo_rez.rs` | Demo mode: a local scene that simulates a server (and its replies to "About Land", to the place profiles, landmarks and history of "Places" and to ExtEnvironment, a library of environments for the picker; `demo_sky.rs`: a rainbow or a 22° halo, with stand-in textures); `demo_stream.rs`: objects and textures that arrive in waves (test of the streaming hitches, AURORA_DEMO_STREAM), `demo_turn.rs`: fast camera turns, again and again (AURORA_DEMO_TURN), and `demo_asset_server.rs`, the local HTTP server its `net=` option downloads the textures from, at the round trip of a distant grid; `demo_arrival.rs`: an arrival in a busy region, objects, meshes and animated avatars all around, every asset downloaded from that server, with the times the loading shows what is in front (AURORA_DEMO_ARRIVAL); `demo_glyphs.rs`: residents whose title, name and message use the extended characters of display names (AURORA_DEMO_GLYPHS); `demo_deform.rs`: an avatar left deformed by a finished pose, to reset (AURORA_DEMO_DEFORM); `demo_link.rs`: our avatar moved by a stand-in simulator through a simulated link (AURORA_DEMO_LINK: ping, jitter, a vehicle to sit on); `demo_tangents.rs`: a low-poly ball with a normal map baked through its own MikkTSpace tangents (AURORA_DEMO_BUMP=sphere); `demo_rez.rs`: objects and an avatar appearing and leaving in front of the camera, for the rez effect (AURORA_DEMO_REZ_EFFECT) |
| `settings.rs`, `keybinds.rs`, `keybinds/layout.rs`, `theme.rs` | Saved settings, shortcuts, Windows keyboard layout and default movement keys, palette; one-shot viewer commands stay out of held controls, while `build/ui/keyboard.rs` shares the build accelerator table between native keyboard ownership and egui dispatch, including logical letters, per-event modifiers and text/web/media history priority |
| `tolerant.rs` | Forward-compatible reading of saved files (settings, account catalogue, HUD sizes, skin): a value this build does not understand costs that value only; a newer build's `settings.json` is written back with what this build does not know; `<name>.newer-backup.json` keeps a repaired file |
| `graphics_presets.rs` | The five graphics levels of Preferences › Graphics (the quality settings each one writes, from the Legacy Viewer's featuretable; the level whose settings match, or none: "Custom") |
| `ui/options.rs`, `ui/options/cx.rs`, `ui/options/pages.rs`, `ui/options/pages/` | The preferences window: a sidebar of pages in sections (Aurora, the modified settings, World and rendering, Communication, Interface, Controls, System) with the search of every setting at its top, the page's tabs, its rows in two columns, the footer (reset everything, reset the page with an Undo, Close). `cx.rs` is the engine of the rows: one declaration per setting (`Cx::row`, `toggle`, `slider`, `combo`, `choice`…, with the settings it edits as `field!` paths) draws it, marks it when it differs from its default (a dot that resets it), finds it for the search (words starting the name, help or choices, accents folded), lists it among the modified settings, counts it for the sidebar and resets it with its page; `subs` puts rows behind the cog of the row before. `pages/` holds one module per page; a test checks that every setting of the former pages still has a row, and that a setting has one page |
| `ui_sound.rs` | Catalogue of the interface sounds (Firestorm's UISnd*), per-sound settings |
| `keywords.rs` | Keyword alerts in chat and IMs (#674, the Legacy Viewer's FSKeywords): the settings of Preferences › Chat › Keyword alerts and the matcher installed process-wide (`set`); the world asks `matches` when a line arrives (`World::keyword_alert` in `world/attention.rs`: sound and system notification), the chat asks `ui/colors.rs::keyword_color` when it draws one |
| `dof.rs`, `app/dof.rs`, `ui/options/pages/graphics.rs` | Depth of field: settings, focus distance and its transition, the lens of a frame (`aurora_render::DepthOfField`); the focus point (camera, pointer, click, lock) and `AURORA_DEMO_DOF`; the Preferences › Graphics group. The effect itself is `aurora-render/src/dof.rs` with `shaders/dof.wgsl` |
| `world/env_edit.rs`, `ui/env_editor.rs`, `app/env_editor.rs`, `world/env_apply.rs`, `app/env_apply.rs`, `world/windlight_presets.rs`, `ui/sky_beacons.rs` | Sky and water editors (LLFloaterFixedEnvironment): the settings map kept as loaded with only the edited keys written back (`env_edit.rs`, also the Windlight XML import), the window with Firestorm's panels (`ui/env_editor.rs`), opening on a snapshot or an inventory item, saving through UpdateSettingsAgentInventory and "Apply only to myself" (`app/env_editor.rs`); the edit slot (ENV_EDIT) is in `world/eep_env.rs`; the day cycle editor (LLFloaterEditExtDayCycle): tracks and key frames with the floater's rules (`world/env_day_edit.rs`), the window with its timeline and the frame panels of the sky and water editor (`ui/env_day_editor.rs`), loading items, frames and tracks, saving and applying (`app/env_day_editor.rs`), the Windlight day import (`env_day_edit.rs`) and its parcel and region contexts (About Land's "Custom"); applying to the parcel or the region (`world/env_apply.rs`: canAgentUpdateParcelEnvironment, the item's flags, a sky or water wrapped in the current day like createDayCycleFromEnvironment, the local textures check; `app/env_apply.rs`: the ExtEnvironment update of the asset or of the settings inline and its answer, the editors' parcel and region contexts, a new day from the default day asset, whose settings map `scene/settings.rs` keeps, the parcel context following About Land's environment, SettingsMakeNoTrans); Firestorm's Windlight presets built in for the day import (`world/windlight_presets.rs`, `assets/windlight/windlight.zip`); the sun and moon beacons (`ui/sky_beacons.rs`, the `sun_beacon` / `moon_beacon` settings) |
| `snapshot/`, `ui/snapshot.rs`, `app/snapshot.rs` | Photos (LLFloaterSnapshot): `snapshot/mod.rs` holds the pure rules (proportions and resolutions up to 7680, file names like the Legacy Viewer's, tile plan and off-centre tile projection, focal length ↔ field of view, settings), `save.rs` writes the file on a worker (PNG row by row with its progress, JPEG, BMP, `.part` then rename), `gallery.rs` lists the latest photos of the folder, `inventory.rs` turns a photo into a texture for the Photo Album (power-of-two sizes, name and description, JPEG 2000 on a worker), `profile.rs` posts one to the profile feed (session cookie from the OpenID cookie, upload configuration, multipart form, 303) with `profile/demo.rs`, the local web profile of the demo and the tests, `quality.rs` the "Photo quality" (the graphics raised for the photo, the user's values set aside), `light.rs` the quick light (time of day, exposure, clouds, worn lights for the photo only), `viewfinder.rs` the viewfinder's logic (display levels, the autofocus that hunts and locks, the zone kept sharp, the distance and exposure scales, the synthesized focus beep, its settings); `ui/viewfinder.rs` is the viewfinder (Preferences › Aurora › Photo window, the default): masks and guide, top bar, light, histogram, level, AF point and face box, distance scale, bottom bar of dials, dock, quick menu, help and the drawer that holds the panel's settings (`SnapshotUi::drawer`), and `ui/viewfinder/controls.rs` its on-screen controls (the lens ring, the AF choices, the camera pad with its sticks, zoom rocker and tilt, the self-timer's countdown), with `app/viewfinder.rs` asking the depth under the AF point (`Renderer::request_pick`), the avatar's face it takes, the beep, the two-stage shutter, the keys and the wheel; the renderer counts its histogram (`aurora-render/src/histogram.rs`) and paints its focus peaking in the depth of field's composite; `ui/snapshot.rs` is the window anchored on the right (or the Legacy Viewer's floater layout, Preferences › Aurora) and the full-frame view (mask, guides, strip), with the price dialogs of the inventory; `app/snapshot.rs` asks the renderer for the photo frames (preview, tiles, freeze frame), stitches them and starts the save or the upload (`NetCommand::UploadAsset` with `upload::Kind::Snapshot`) |
| `ui/appearance.rs`, `ui/appearance/items.rs`, `ui/appearance/gallery.rs`, `world/appearance.rs` | Appearance window, gallery / outfits / worn, editing and Save as dialog; menus of the items and of the gallery, save / delete confirmation, image choice, renaming; attachment points / HUD, profile and original; rules for changing the COF, saving through links, protection of the body parts, offline scenario |
| `aurora-net/src/outfits.rs`, `aurora-net/src/outfits/categories.rs` | Writing of the outfit links, favourites, names and images through AIS InventoryAPIv3, move to the trash; confirmation and read-back of the folders / items |
| `aurora-net/src/freeze.rs`, `release_notes.rs`, `sim_console.rs`, `voice_moderation.rs`, `postcard.rs`, `session/sim_requests.rs`, `world/sim_console.rs`, `world/voice_moderation.rs`, `ui/sim_console.rs`, `demo_sim_extras.rs` | The small simulator features: being frozen (ViewerFrozenMessage, the state shown by a snowflake among the parcel icons, and the AvatarFrozen alerts worded by the viewer); Help › Server release notes (ServerReleaseNotes, its `Location` opened); Advanced › Region console of estate managers (SimConsoleAsync, its output from the event queue's SimConsoleResponse); nearby voice moderation (SpatialVoiceModerationRequest: who may moderate, the entries under a resident's voice row, the notice when a moderator mutes us); a photo by e-mail (SendPostcard, the photo window's E-mail destination in `ui/snapshot.rs` and `app/snapshot.rs`); their demo (`AURORA_DEMO_FREEZE`, `AURORA_DEMO_SIM_CONSOLE`, `AURORA_DEMO_VOICE_MODERATION`) |
| `slurl.rs`, `slapp.rs`, `link_trust.rs` | Links in texts: SLURL (navigation bar, "Region (x,y,z)" labels of the place links); the other `secondlife:///app/...` commands (`slapp.rs`: group, parcel, objectim, inventory, experience, classified, event, map, chat, search, agent verbs, the ones refused from a text received; their labels and menus in `ui/chat/app_links.rs`, what a click runs in `app/app_links.rs`, the remote object inspector in `ui/remote_object.rs`); trust of web links (trusted site, unknown, dangerous: shorteners, fake official names, IP addresses…), judged locally on the URL |
| `link_preview.rs` | Link previews of the conversations (issues #538, #739): which links of a message get a picture under them (a picture's file, a YouTube video) and whether it loads by itself (Preferences › Chat: trusted sites by default, the others after a click; never a link that hides its destination), the download (no cookie or referrer, redirects kept on trusted sites, 8 MB at most) and the decoding off the frame (PNG, JPEG, GIF frames, WebP, BMP, kept at the size drawn), YouTube's oEmbed title, the loopback page that holds a video's player (YouTube's player wants a referrer), and the demo's pictures made without network |
| `ui/search.rs`, `ui/search/details.rs`, `ui/search/aurora.rs`, `ui/search/aurora_details.rs`, `ui/place_card.rs`, `world/search.rs`, `aurora-net/src/search.rs`, `aurora-net/src/session_search.rs`, `demo_search.rs` | "Search" (the Legacy Viewer's search): directory queries and their flags, reading of the replies and of the event / group details (`aurora-net/src/search.rs`, pure; sending and receiving in `session_search.rs`); one result list per tab with the pending query, pages of 100, statuses, sorting and selection (`world/search.rs`); window with seven tabs, forms, sortable list and web page in the built-in browser (`ui/search.rs`, the Legacy look), detail cards and their actions (`ui/search/details.rs`); Aurora's look, the default: one field, type chips with their counts, the chip's filters on one line, rich rows and the sections of "All" (`ui/search/aurora.rs`), the details pane beside them (`ui/search/aurora_details.rs`); the place card shared with the windows that list places (`ui/place_card.rs`: snapshot, name, region, rating, trailing figures; the rows of Aurora's Places, their favourites dragged by a grip); offline directory of the demo (`demo_search.rs`, AURORA_DEMO_SEARCH) |
| `ui/group_profile.rs`, `ui/group_profile/`, `world/group_data.rs`, `aurora-net/src/groups.rs`, `aurora-net/src/session_groups.rs`, `demo_group.rs` | Group profile: LLGroupMgr's data of a group (members by the GroupMemberData capability or GroupMembersRequest, roles, role / member pairs, titles, notice list; replies and request ids in `aurora-net/src/groups.rs`, pure; sending and receiving in `session_groups.rs`), kept per group with stale replies dropped (`world/group_data.rs`); window in the Aurora look (`group_profile/view.rs` header and tabs, `home.rs`, `members.rs` with `admin_members.rs` and the abilities catalogue `abilities.rs`, `notices.rs`, `invite.rs`, `administration.rs`) or the classic one; "Create a group" (`create.rs`: LLPanelGroupCreate's rules, the profile's header and the identity cards of `administration.rs`, or Firestorm's creation panel; CreateGroupRequest and its name rules in `aurora-net/src/groups.rs`); demo group server (`demo_group.rs`, AURORA_DEMO_GROUP_PROFILE, AURORA_DEMO_GROUP_CREATE) |
| `ui/find_shortcut.rs` | Ctrl+F requested by three features: the single rule (`owner`) that gives it to the window one of whose fields has the keyboard (script editor, conversation), otherwise to the conversation under the pointer, otherwise to the viewer's shortcut ("Search"); the windows with a search bar declare themselves to it every frame (`take`), the application queries it before running its shortcut (`window_takes_it`) |
| `ui/close_key.rs` | Ctrl+W closes the frontmost window, as File › Close Window: taken at the start of the interface's frame, before a text field reads it (`begin`), only when a window is there; every `widgets::Floater` declares itself (`shown`) and closes as its cross when the key is for it (`take`); the Conversations window takes it first for its conversation shown |
| `ui/widgets.rs`, `ui/widgets/resize.rs`, `ui/widgets/cascade.rs`, `ui/widgets/demo.rs` | Shared widgets and floating windows; Firestorm inventory preview/editor cascades by type and creation order; collapsing by double-clicking the title or by a button, expanded size kept and collapsed bar movable; offline gestures on the real title; resize handles bounded before the move, opposite edge fixed, limits of the content and of the visible area; position and size kept by egui; names longer than their space cut with "…" with the whole name on hover (`elided_label`, `paint_elided`) for the lists of residents, groups and conversations |
| `ui/layout.rs`, `ui/layout/anchor.rs`, `ui/layout/gesture.rs` | Interface layout: egui's memory (positions, sizes, collapsed windows) written to `ui_layout.ron` on exit and read back at startup; work area left by the bars (`work_area`) and common gap `GAP` of the default layout and of snapping (none: the windows touch); `Slot`, through which `widgets::Floater` and the minimap (`widgets::bare_window`) go, records each measured window and its movement room; `anchor.rs` converts existing default positions/sizes and older layouts directly into the closest of eight corresponding perimeter points and its signed offset. A window can supply an explicit first-opening point. Completed moves/resizes recompute the closest pair among four corners and four side midpoints. Its invisible point and signed offset persist across viewer resizing and sessions, including closed windows and migration of older layouts; glued windows share a point chosen from their block bounds. `gesture.rs`: the gestures of the frame (`layout::frame`, `layout::resizing`): window dragged by its title together with those stuck to it, block kept within the screen, snapping proposed then applied on release (under the pointer for a resize), sticking links created, saved with the layout and removed when they no longer hold (window closed, collapsed, moved apart, Alt) |
| `ui/theme_guard.rs` | Test only: reads the interface sources (`ui/`, `build/ui/`, `imports/ui*`, `crash/window.rs`) and fails if a window directly uses an egui widget replaced by a shared widget of `ui/widgets.rs`, or itself writes a radius or a colour instead of those of the theme (AGENTS.md, section 7); list of the rules, of their replacement and of the exceptions (files of `ALLOWED`, lines marked `// theme-guard:`), themselves checked |
| `ui/snap.rs`, `ui/snap/overlay.rs` | Window snapping, geometry only: which moving edges stick to what (edge facing another window, same-side edge of a neighbour, work area; 10 px margin, edges that touch (`layout::GAP`, no gap), size limits of a resize, holding of the displayed snap), guide to draw, side-by-side windows (`touching`), groups (`group`, `block_limits`). `layout/gesture.rs` only snaps and attaches moved windows while Alt is held, and Alt also detaches the dragged window from its group; resize snapping retains its separate behavior. `overlay.rs`: the guide and the "Alt" pill drawn above the windows |
| `ui/text_menu.rs`, `ui/text_menu/tests.rs` | Right-click menu of every text field, like Firestorm's text editors (menu_text_editor.xml: Cut, Copy, Paste, Delete, Select All, greyed by canCut / canCopy / canPaste / canDoDelete): one hook around the interface pass, no change at the fields. It finds the focused field with a `TextEditState` under the right click, reads editable / password from the IME request egui makes for it, keeps a selection right-clicked inside it, and plays the chosen entry into the field at the next frame as egui events (the clipboard's text for Paste). A field with its own right-click menu (script editor, notecards) keeps it. Demo `AURORA_DEMO_TEXT_MENU` |
| `ui/numeric_drag.rs` | Continuous dragging of the numeric fields, including those of the sliders: virtual pointer fed by relative movement, pointer and native anchor held in physical pixels so UI zoom adds no motion; release, loss of focus and disappearance of the widget free the pointer. Keyboard entry and slider bars unchanged |
| `ui/options/scale.rs` | Live interface scaling: native input sampled before egui's pending zoom and numeric pointer rewriting, gesture origin and sensitivity fixed in physical pixels while the control resizes. Percent display/input with the default 100% at the track midpoint and the established 75–200% limits; saved factors unchanged, numeric text commits on Enter or loss of focus. `OptionsResult::ui_scale_changed` takes a dedicated zoom action in `app.rs`, without reapplying render/network/theme settings or synchronously saving the unchanged crash record at each movement |
| `ui/hud.rs`, `ui/nameplate.rs`, `ui/nameplate/mesh.rs`, `ui/nameplate/paint.rs`, `ui/nameplate/tests.rs` | Name tags above avatars and floating texts. `Projector::for_ui` samples the scale inside the UI pass, after pending zoom is applied, so projection and painting use the same scale. `hud.rs` chooses the tagged avatars, their anchor point and what is known about them (names, status, voice, complexity), and draws the classic name tag. `nameplate.rs` is the name plate (default style, `Settings::classic_name_tags` for the old one): pure functions for the level according to the distance with its hysteresis (`level_for`), the lines, badges and status dot of each level, the measurements taken from the mock-up (`Metrics`) and the layout (`layout`), the voice envelope and its halo, and `Plates`, what each plate keeps from one frame to the next (level and transition, envelope, photos requested at most four per second, geometry). `mesh.rs` builds the geometry as simple meshes — pill, border, shadow and halo as rings around an outline, antialiased discs and rings — instead of egui's rounded shapes, tessellated again every frame; `paint.rs` draws a plate, keeps its geometry as long as its state does not change and redraws it without a copy when it has not moved |
| `world/lookat.rs`, `ui/lookat_targets.rs`, `ui/options/pages/privacy.rs`, `ui/options/pages/avatars.rs` | Look-at (LLHUDEffectLookAt): `lookat.rs` keeps our target with its priorities and timeouts, the targets the other avatars send, the preferences of the Legacy Viewer's Privacy › LookAt and Avatar panels (`lookat::Settings`, migrated from the former single switch) and the pure rules of what the others get (`outgoing`: nothing, a dull target or ours) and of the distance limit (`within`); `app.rs` (`set_look_at`, `update_look_at`, `sync_static_eyes`) applies them. `lookat_targets.rs` draws the targets, names and lines on the world layer (Aurora's halo, or the Legacy Viewer's crosshair as the legacy `lookat::Style`), hidden by the scene like the name tags (`aurora_render::world_label_at`, lines cut into pieces by their length on screen). `options/pages/privacy.rs` has what we send of where we look; the others' targets and the head-follows-mouse sliders are on the Avatars page; the halo / crosshair style and the aim (point under the cursor, or the Legacy Viewer's direction: `lookat::Aim`, `cursor_look`) are on the Aurora page |
| `world/object_im_filter.rs`, `ui/options/pages/privacy.rs`, `demo_object_im.rs` | Objects' messages from elsewhere (issue #623): the location of an IM_FROM_TASK binary bucket ("Region/x/y/z") read and compared with our region and parcel (`landmarks::in_parcel`); llInstantMessage from another region or parcel and llRegionSayTo from outside the parcel dropped when Preferences › Privacy asks for it, our own objects and (by choice) friends' objects let through. |
| `ui/options/pages/aurora.rs` | Preferences › Aurora: what Aurora does differently from the Legacy Viewer, each redrawn element with its way back to the Legacy look (windows, the world and the interface, gaze, movement, animation timing); `set_all` sets every look at once (and Aurora's water); the entry of the sidebar carries the aurora gradient |
| `ui/fonts.rs`, `ui/fonts/fallback.rs` | Interface fonts: main font, embedded (Inter by default, Noto Sans, Roboto, each with its semi-bold face) or chosen, those of egui, then system text fallback fonts before egui's icon font for characters the main faces do not have (display names, titles, chat: Georgian, CJK, Arabic, mathematical letters…). `fallback.rs`: list per script, first installed font that draws the probe character, files memory-mapped and checked on the `aurora-fonts` thread, alignment of their baseline with the main font; installed on the next frame if the search was not finished. `AURORA_FONT_FALLBACK=0` removes them |
| `ui/emoji.rs`, `ui/emoji/codes.rs` | Colour emoji: the built-in font's pictures, the text fields that draw them, and the picker (its search takes the keyboard, Escape is its own). `codes.rs`: the `:code:` shortcodes of the chat fields, converted when the closing `:` is typed, a list of suggestions from `:` and two letters (arrows, Tab / Enter, Escape), and the same rules for what is sent (not inside a word, a time or a link). |
| `ui/exit.rs` | Translated exit modal: quit, log out to the login screen, cancel; background blurred on the GPU and interactions blocked; non-blocking wait for the end of the session, with Firestorm's six-second fallback timeout |
| `updater.rs`, `ui/update.rs` | Asynchronous controller and update window: notes, progress, restart and errors; manual check in Help / login, ignored tag and Ask / Automatic / Required only choice. Isolated synthetic demo through `AURORA_DEMO_UPDATE`. |
| `ui/news.rs`, `ui/news/markdown.rs`, `links.rs` | News of the login screen: Markdown files `news/<language>.md` of the repository, one per interface language (fallback to the base language, then English). At startup, the last download kept in the cache (`news-<language>.json`) or, failing that, the files compiled into the viewer; then the file from `main`, downloaded in the background (bounded size, 20 news items, texts cut, `https://` links only), replaces them. Reloaded when the interface language changes; a failure leaves the panel as it is. No download in demo nor in the test installation of updates; `AURORA_NEWS_URL` reads a local server or another branch, without cache. `markdown.rs`: tolerant reading of the format (`## date — title`, text, link), HTML comments ignored |
| `ui/inventory.rs`, `ui/inventory/view.rs`, `ui/inventory/controls.rs`, `ui/inventory/context.rs`, `ui/inventory/clipboard.rs`, `ui/inventory/delete.rs`, `ui/inventory/dialogs.rs`, `ui/inventory/properties.rs`, `ui/inventory/thumbnail.rs`, `app/inventory.rs`, `app/inventory_thumbnail.rs` | Inventory: pure sorting / filters and per-generation view cache in `view.rs` (index, sorting and filtering in parallel with rayon), persistent commands and preferences in `controls.rs`, filtered tree including the parent folders, expansion states per tab, results drawn according to the visible area without a limit of 500 and status area of fixed height; scrolling on both axes, width of the names measured in the visible area then kept for the virtual results, commands adapted to small widths; menus per type, multiple selection with grouped add / detach, renaming in the row after creation or on request, folder windows limited to their descendants with a filter of their own, clipboard shared between windows and native Ctrl+C / Ctrl+X / Ctrl+V shortcuts according to the focus of the list in `clipboard.rs`, Delete and bottom-of-window trash through the shared deletion in `delete.rs`, properties and confirmations, previews / editing of notecards and scripts, reading of the animation statistics; image editor with six actions, tree picker and screenshot of the scene without the interface; decoding / resizing / encoding off the render thread and execution of the actions after the interface is drawn |
| `world/inventory/actions.rs`, `world/inventory/wearable.rs`, `world/inventory/demo.rs`, `world/inventory/preview.rs`, `world/inventory/tests.rs` | Permission rules, moves / copies / links and offers, preparation of the Marketplace listings, default clothing assets and synthetic scenario; v3 inventory cache with full permissions and AIS favorite metadata |
| `aurora-net/src/inventory/operations.rs`, `aurora-net/src/inventory/thumbnail.rs`, `aurora-net/src/session/inventory_upload.rs` | AIS mutations with read-back, remapping of the UUIDs assigned by the server, UDP creations / copies and acknowledgements with expiry, saving of the documents through capabilities; AssetUpload / Xfer upload of clothing before the item is created; free thumbnails through InventoryThumbnailUpload, JPEG2000 POST then AIS and read-back |
| `aurora-assets/src/j2k/encode.rs` | Memory-bounded OpenJPEG encoder, square RGB / RGBA thumbnails of 64 to 256 pixels and alpha preserved |
| `logging.rs`, `cache.rs`, `credentials.rs` | Logs (the capture of the lines one thread logs, and in memory the last warnings and the recent output of the diagnostics, for the Dev Toolbox), disk cache, remembered password (OS vault) |
| `dev_toolbox/`, `ui/dev_toolbox.rs` | The Dev Toolbox (Advanced › Dev Toolbox, Ctrl+Alt+D; the former Preferences › Debug page): every tool with its stable identifier (J2, L1, R3…), kind, icon and group, the layout the user arranges and its saved form (new tools join their group, removed ones stay out), the anonymisation of what is copied ("Avatar 1", "Me") and the header of a copy (`dev_toolbox/`); the window: target bar, groups dragged in Customize, "Send a diagnostic", the last warnings (`ui/dev_toolbox.rs`). The agents' guide is docs/DEV_TOOLBOX.md; `AURORA_DEMO_TOOLBOX` |
| `cache_location.rs`, `app/folders.rs` | The cache and transcript folders chosen in the preferences (CacheLocation, InstantMessageLogPath): `cache_location.rs` reads the cache's choice from the settings file at startup, before anything uses the cache, and removes the previous cache when the folder changed (only the viewer's own folders); `app/folders.rs` runs the folder dialog off the frame, and moves the account's transcripts at once through `chat_log` |
| `settings/backup.rs`, `app/settings_backup.rs`, `ui/options/backup.rs` | Back up and restore the settings (the Legacy Viewer's Backup tab), Preferences › Language and updates › Backup: `settings/backup.rs` is the one-file format (the settings without the login nor this PC's window, folders, devices and machine id, the windows' layout, the skin's JSON files, each account's preference files by a fixed list of names) and the restore that keeps this PC's values; `app/settings_backup.rs` runs the file dialogs, the reading and the writing off the frame, then puts back the layout, the skin and the accounts' files (not the logged-in account's); `ui/options/backup.rs` is the confirmation with the parts to restore, which swaps the settings as the undo of a reset does |
| `world/open_ims.rs` | The private conversations open at the end of a session, kept per account (`open_ims.xml`, FSLastOpenIMs) and reopened at login when FSRestoreOpenIMs is on |
| `world/region_alerts.rs` | The region's script count change (SimStats, nearby chat) and simulator version change (AgentMovementComplete, a notice) alerts of Preferences › Alerts |
| `world/collisions.rs`, `ui/bumps.rs` | Bumps, pushes and hits (MeanCollisionAlert): the last 20, one per resident and kind, newest first, the optional nearby chat line and report to the agent's scripts (`world/collisions.rs`), and the Help menu's "Bumps, pushes & hits" window with its copy and avatar menu (`ui/bumps.rs`, LLFloaterBump) |
| `movement/crouch_toggle.rs` | Crouch as a toggle (FSCrouchToggle): the Down key's latch, let go by a jump or flying |
| `world/chat_log.rs`, `world/chat_log/`, `world/transcripts.rs` | Conversation history on disk, like Firestorm (LLLogChat, LLIMModel::logToFile). `chat_log/format.rs`: text of a line (`[YYYY/MM/DD HH:MM]  Name: text`, SLT time, following lines preceded by a space, `:` of a name written `%3A`) and its reading back; `names.rs`: file of each conversation (`chat`, legacy name of the resident, `<group> (group)`, dated conference), forbidden characters, reserved names of Windows, date in the names; `store.rs`: folder of an account on a grid (`accounts/<uuid>/chat_logs/<grid>`), `aurora-chatlog` thread that appends the lines (file opened, written and closed again for each batch, each task carrying its full path) and reads back the end of a file (LOG_RECALL_SIZE); `transcript.rs`: whole file for the History window, in pages of 100 messages, without keeping it in memory (index of the pages, page-by-page search); `chat_log.rs`: facade opened at login and closed at logout, settings. `transcripts.rs` links the conversations to the files: file name of a session, lines waiting for a name, history requested before the first write, history lines (`Origin::Log`, `Origin::ServerLog`) placed before the new ones, merge with the history of the group chat server, stable numbering of the lines for the view (`LineClock`) |
| `ui/notifications.rs`, `ui/notification_center.rs`, `ui/notification_center/presence.rs`, `ui/friend_prompt.rs`, `world/notifications.rs`, `world/notification_history.rs` | Notifications: the list and its answers (`world/notifications.rs`, what leaves it recorded with its outcome), Firestorm's toasts and bell next to the wolf and the shared cards (`ui/notifications.rs`), Aurora's center (`ui/notification_center.rs`: cards unrolling from the bell at the right end of the top bar and folding back into it (with the islands exactly under the system and bell islands, `NotifUi::column`), "To do" and "History" tabs, paused bell in do not disturb; `presence.rs`: the friends' online / offline toasts at the place chosen in Preferences › Chat, laid out in the room the bars leave (`ui::layout::bars_room`) above the nearby chat lines, waiting at the top right while the cards or the list are out), chosen by Preferences › Aurora › Notifications; the history kept per account (`notification_history.json`, 30 days or 500 entries, written on a worker thread, read with the tolerant reader); `ui/friend_prompt.rs`: a friendship offer as a card in the middle of the screen instead of its toast (#465, Preferences › Chat › Toasts, on), answered through the same list |
| `world/region_restart.rs`, `ui/region_restart.rs`, `app/region_restart.rs`, `aurora-net/src/region_restart.rs` | Region restart alert (LLFloaterRegionRestarting): the simulator's notice read from AlertInfo `RegionRestartMinutes` / `RegionRestartSeconds` or OpenSim's `/RESTART_X_…`, and the cancellation (`aurora-net/src/region_restart.rs`, pure); the countdown, its steps (5 min, 1 min, 30 s, 10 s), its sound once per step, its shakes, minimized / closed / back at 30 s, cancelled or left (`world/region_restart.rs`); the alert card in the top third of the view, its pill, the landmark menu, the managers' line and its confirmation, the bell's pinned card, Firestorm's yellow box as the legacy look (`ui/region_restart.rs`); the glue: sound, taskbar flash, Firestorm's view shake (`Camera::shake`), nearby chat line, channel announcement, `EstateOwnerMessage` "restart" -1, the order of the layers (above the toasts, below the open center), the reconnect after the restart, `AURORA_DEMO_RESTART` (`app/region_restart.rs`) |
| `ui/link_health.rs`, `app/link_health.rs` | The link with the agent's region as the user feels it: the pill while the region is silent for 5 s or more (blip, lagging simulator, waking laptop) and its count, read from the main simulator's last packet (`NetStats::simulator_motion`); after a lost link (`NetEvent::Disconnected { lost: true }`: silence of 100 s, event queue given up, region gone, not a kick) the login screen offers to go back to the region and the spot the user was in (`reconnect_place`, the login's `uri:` start, for the same grid and account only); `AURORA_DEMO_LINK` (`app/link_health.rs`) |
| `world/calls.rs`, `ui/chat/call.rs`, `ui/call_invite.rs`, `ui/context/multi.rs`, `voice.rs`, `demo_calls.rs` | Voice calls with a resident, a group or a conference (LLVoiceChannelGroup, LLIMMgr::startCall / inviteToSession): `world/calls.rs` holds the call in progress and its state, asks the credentials (ChatSessionRequest through `NetCommand::Call`), handles the invitations, their notification and the errors; `voice.rs` joins the call's "multiagent" channel instead of the spatial one while it lasts and reports its state and participants; `ui/chat/call.rs` is the strip under the conversation's head (state, timer, participants, microphone, add, hang up); `ui/call_invite.rs` adds residents to a conversation (picker, residents dropped on it, the confirmation that turns a conversation with one resident into a conference); `ui/context/multi.rs` is the menu of several residents chosen in a list, independent of how the list is drawn; `demo_calls.rs` answers the requests and stands in for the voice server in the demo (`AURORA_DEMO_CALL`) |
| `ui/presence.rs` | Status of a resident and the color and shape of its dot, shared by the name plates, the lists, the profiles and our own status: in our regions from the away / do not disturb animations, else a friend's online status, else an automatic response until the next message, else unknown. The shape (`Shape`, `paint_mark`: a disc online, a crescent away, a barred disc in do not disturb, a hollow ring offline) says the status without its colour (#418) |
| `colorblind.rs`, `ui/options/pages/accessibility.rs` | Colour blindness (#418, Preferences › Accessibility): the deficiency and its strength, process-wide (`set` / `current`), read by `Theme::palette`. `palette` moves only the colours that carry a meaning and collide for the deficiency, to an Okabe–Ito hue of the same luminance (success blue for red–green, warning orange and links sky blue for blue–yellow, a ladder of greys for monochrome); `adapt` daltonises the other colours (`ui/colors.rs`, the complexity ramp of the name tags) from the simulation of Machado et al. 2009. `options/pages/accessibility.rs` is the page, with a preview of the status dots |
| `ui/chat/input_history.rs` | Sent-message recall per conversation, shared by both nearby chat fields, with independent browsing and drafts per editor. Ctrl+Up/Down applies in every chat style; in memory, without a count limit, consecutive duplicates skipped. |
| `ui/chat/keys.rs` | The keyboard of the Conversations tabs (#651), in every chat style: Alt + the arrows go round the conversations as the list shows them while the keyboard is in the window; Ctrl+W closes the conversation shown and shows the next one, or the window on a tab that cannot be closed. |
| `ui/chat/latest.rs` | The button back to the latest messages (#558), in every chat style: it fades in over the bottom of a conversation's lines once the reader is away from the end (`view.rs` keeps how far, and from which line the messages are new), says how many came in since, and puts the scroll offset past the end, where the scroll area sticks to it again. |
| `ui/chat/flash.rs` | The soft violet flash of a conversation's entry when a message comes in while another is shown (eight rises over four seconds, then a faint tint until read), shared by the column and the tabs; Preferences › Chat. |
| `ui/chat/reorder.rs` | In-memory presentation order of open conversations, shared by all chat styles; category-bound row dragging, insertion feedback, edge scrolling and cancellation. Does not reorder network sessions or move pinned entries. |
| `ui/chat.rs`, `ui/chat/view.rs`, `ui/chat/bubbles.rs`, `ui/chat/header.rs`, `ui/chat/find.rs`, `ui/chat/history.rs`, `ui/chat/invite_card.rs`, `ui/chat/offer_card.rs`, `ui/chat/previews.rs`, `ui/chat/select.rs`, `ui/chat/workspace.rs`, `ui/chat/sidebar.rs`, `ui/chat/conv_menu.rs`, `ui/chat/line_menu.rs`, `ui/chat/head.rs`, `ui/chat/stage.rs`, `ui/chat/thread.rs`, `ui/chat/members.rs`, `ui/chat/compose.rs`, `ui/chat/typing.rs`, `ui/chat/console.rs` | Conversations window and the nearby chat lines above the chat bar (`console.rs`: Aurora's cards grouped by sender, beside the Legacy Viewer's one-line look of `chat.rs`'s `toasts`). Aurora's workspace (the `Aurora` and `Compact` styles, `ChatStyle::workspace`; the Bubbles and Legacy styles keep the window of tabs): `workspace.rs` lays it out and keeps the window still on its background; `sidebar.rs` the column of conversations (search that filters and starts, Contacts and nearby chat pinned, private / groups / conferences sections with their "+", entries with status shape, sub-line and unread, rail of pictures below 600 px, the panel of a call shown elsewhere); `conv_menu.rs` the right-click menu of a conversation of the list, the column's and the tabs' (Close, its call, the resident's menu of a name or the group's actions, the transcript); `head.rs` the 48 px head (picture, title, sub-line, call, profile, members, find, menu); `stage.rs` the calls (incoming banner, stage with speakers and volumes, its one-line strip, the call going on without us); `thread.rs` the messages grouped by author (picture gutter) or in compact rows, with tags, hover toolbar (copy, quote, profile, menu), mentions and the "New" line; `members.rs` the members' panel of a group or conference; `compose.rs` the field (typing line, emoji, gestures or inventory, channel, whisper / say / shout, send); `typing.rs` our IM_TYPING notices (start / repeat / stop, FSFloaterIM) and the "is typing" text. `bubbles.rs`: the lines of the IMs and group chats as a messenger shows them (bubbles on either side, grouped by sender, day between days, notices in a pill); `header.rs`: head of a conversation (picture, status, names, actions and their menu); `line_menu.rs`: transcript-owned context menus retain their original message and sender independently of row visibility while live updates and scrolling continue; moving the containing window dismisses them; `view.rs`: only the visible lines are laid out (heights measured and kept per width, off-screen lines measured without being drawn within a per-frame budget, scroll offset corrected when a line changes height above); `find.rs`: toolbar filter and Ctrl+F search bar specific to each conversation; `history.rs`: History window (pages, search in the whole file, folder); `invite_card.rs`: card of a teleport offer or request in the sender's conversation, with its buttons or its answer (its frame shared by the cards); `offer_card.rs`: card of a resident's inventory offer in the giver's conversation, Keep / Reject or its answer; `previews.rs`: the cards under links in the Aurora, Compact and Bubbles styles (picture with open / copy / hide, animated GIF, blurred group pictures, a card that asks before loading, too large; a YouTube video playing in its card or in the player window, through the media plugins' browser; one video at a time, stopped when its card is no longer drawn); `select.rs`: text selection across the lines (Conversations and History), held as line number + character so that it outlives the lines laid out only in sight: the pieces of text drawn in a line (`label`, `atom`, used by the emoji runs and the links) are recorded with their place and painted selected, the pointer finds its character among them, Ctrl+A / Ctrl+C copy as the Legacy Viewer's transcript lines |
| `aurora-net/src/offline_im.rs`, `aurora-net/src/session_offline.rs` | IMs kept by the grid during the absence: decoding of the reply to `ReadOfflineMsgs`, request made once after login (capability, otherwise `RetrieveInstantMessages`), replies through capability to the friendship offers and group invitations received offline |
| `bug_report/`, `ui/bug_report.rs`, `ui/bug_report/tests.rs`, `app/bug_report.rs` | **Disabled, out of the build**: "Report a bug" opens GitHub's issue chooser in the browser (`links::NEW_ISSUE`); to bring the form back, uncomment the lines marked "Bug report form (disabled, see main.rs)". Bug report: explicit snapshot of the authorised data, form with Description / Picture / Data tabs, localised values that can be unchecked, export with English titles; public diagnostics with a closed vocabulary and images re-encoded off the main thread. Local export with separate public / private folders; prefilled GitHub link with clipboard fallback, or issue creation after the Device Flow of a GitHub App. `uploads.rs` declares the hashes and sends the public image and the optional private ZIP to the PHP / Cellar API, then adds the links; partial failure resumed on the same issue. The GitHub authorisation stays only in memory in `github.rs` and is not passed to the Aurora API. No SL login secret, chat or IM enters the public model. |
| `report_abuse.rs`, `ui/report_abuse.rs`, `app/report_abuse.rs`, `aurora-net/src/abuse_report.rs`, `aurora-net/src/session/abuse_reports.rs` | Report Abuse, like LLFloaterReporter: `report_abuse.rs` holds the rules (built-in categories, the checks and the copyright notice of onClickSend, the summary and details of gatherReport, the screenshot scaled and encoded off the main thread); `ui/report_abuse.rs` the window; `app/report_abuse.rs` its targets (menu, avatar, object, chat line), the object picker, the screenshot taken without the window and the sending. `aurora-net/src/abuse_report.rs`: the report's LLSD, the AbuseCategories request, SendUserReportWithScreenshot (the report as the body of the screenshot's upload) and SendUserReport; `session/abuse_reports.rs` the object's RequestObjectPropertiesFamily (COMPLAINT_REPORT_REQUEST) and, without these capabilities, UserReport after the screenshot's legacy upload (`session/inventory_upload.rs`). |
| `screen_rate.rs` | Refresh rate of the screen under the window, read again once per second (window moved, screen mode changed): cap of the login screen and frames already paced by vertical sync |
| `window_hold.rs` | Frames while Windows holds the window (mouse button pressed on the title bar, a border or a title bar button, move, resize, system menu): Windows follows those in a message loop of its own, in which winit's loop does not turn and no redraw is asked. The window procedure is hooked to know when a hold begins and ends, and a helper thread then asks for an immediate repaint, which winit delivers as a `RedrawRequested` on the main thread. During a hold the frame limiter does not wait on the main thread (`paces`): the helper waits between two frames, and spaces them out while the cursor moves so that the window keeps following it |
| `alt_codes.rs` | Windows Alt codes in text fields: left Alt + numpad digits read from the key events while egui has a text field focused (the digits kept from it), the character given at Alt's release in the OEM code page, or the ANSI one after a leading 0 (winit drops the `WM_CHAR` Windows sends after the Alt key-up) |
| `world/attention.rs`, `app/attention.rs`, `system_notify.rs` | Taskbar flash and system notifications while the window is not in front (LLWindow::flashIcon, the Growl manager): what asks for attention, recorded by the world beside its notifications (`world/attention.rs`); the settings, the focus, do not disturb, the one-second throttle by title, the flash (FlashWindowEx between frames) and the click back to the conversation (`app/attention.rs`); Windows notifications from a worker thread, with the AppUserModelID registered for the current user (`system_notify.rs`) |
| `window_show.rs` | Showing the startup window once its first frame is drawn: on Windows one `ShowWindow`, already maximized when it has to be, then a foreground and focus request that follows Windows' rules (LLWindowWin32::show), then winit's flags, visible first (winit maximizing a window it thinks hidden showed it, activated it and hid it again, leaving it without the keyboard focus) |
| `frame_profile.rs` | Frame profile (AURORA_PROFILE): time of each step of the frame, summary line per second in the log (average and maximum of each step, number of slow frames; detail of the streaming on the main thread, `s_*`, of the frame parameters, `p_*`, and of the interface per group of windows, `u_*`; time spent off the CPU per step, `off_cpu:`; meeting of the main thread and the render thread, `thread` segment) |
| `net_profile.rs` | Network profile: what the link and the session did over each second, from the counters of `aurora-net` (`NetStats`: packets, resends, sequence gaps, longest pause of the session loop, delay between a key and its AgentUpdate), the wait of the events before the main thread applies them and the corrections and prediction lead of our avatar (`agent.rs`, `own_motion.rs`). Read by the Performance window; one `net summary` line per second in the log with AURORA_PROFILE |
| `deep_profile.rs` | Deep profile (AURORA_PROFILE): every 10 s, the `msg summary`, `http summary`, `asset summary` (decode and mesh job durations, queues, GPU uploads, disk caches), `avatar summary` and `memory summary` lines, from the probes of `aurora-net` (`probe.rs`), of the background jobs and of the scene |
| `sys_probe.rs` | What the system knows of the process (Windows): its memory, and the CPU time of its threads by group, written as `cpu summary` every 10 s by a thread of its own (AURORA_PROFILE); while the resource monitor is open, a sampler thread (`aurora-monitor`) reads every half second the threads' groups with their priority, the load of each logical core (the system's `\Processor(*)` counters) and the media plugins' processes with those they started |
| `disk_io.rs` | Bytes the viewer reads from and writes to its own files, by origin (texture, mesh and object caches, inventory, logs, the rest), counted where the files are touched: the resource monitor's disk rates |
| `ui/monitor/`, `app/monitor.rs` | The resource monitor, the Performance window's full view (`ui/perf.rs` keeps the compact one and the switch between them): `mod.rs` the rail of eight sections, the status bar, the CSV report; `model.rs` what each part of the viewer costs in processor, frame time, GPU, memory, video memory, network and disk, attributed from the thread loads, the frame's steps, the GPU timers, the probes and the disk counters (nothing measured stays unknown); `snapshot.rs` the half-second snapshot and a minute of history; `draw.rs` its cards, graphs, segmented bars and tables; one file per section (`overview.rs`, `subsystems.rs`, `threads.rs`, `memory.rs`, `network.rs`, `disk.rs`, `scene.rs`, `region.rs`); `app/monitor.rs` turns the measures on while the view shows (frame profile and probes without a log line, the sampler), gathers the snapshot, lists the scene's costliest avatars and objects and answers the monitor's actions |
| `scene/lights.rs` | Local lights of the frame: the prim lights closest to the camera (range × 1.5, falloff, colour) and, for those that have a texture, their projector — cone computed from the position, the rotation and the size of the prim (`aurora-render` `Projector`), texture reserved as long as the light stays in the list; choice of the two projectors that cast shadows (the most present on screen) and fade of their shadow maps (`SpotShadowSlots`) |
| `scene/animesh.rs` | Standalone skeletons of the animated objects, animations of the linkset, bounds of the poses for culling and shadows, demo scenario |
| `scene/body_physics.rs` | Avatar body physics (LLPhysicsMotion): six springs per avatar wearing a physics layer, driven by the world motion of its chest and pelvis at a fixed step, move the LEFT_PEC / RIGHT_PEC / BELLY / BUTT collision volumes of the posed palette (what mesh bodies are rigged to), in the parallel pose loop; detail and on/off from Preferences › Graphics |
| `scene/morph.rs` | Morphs of the system avatar body: the visual parameters that move the vertices of the base meshes (sex, body fat, face, system hair, eyelashes, skirt). A body part is built by a job for a set of weights and shared by the avatars that have them (`GeomKey::AvatarMorph`); hidden parts are not built |
| `scene/skin_binds.rs` | Skin bindings sent to the GPU (inverse matrix and palette bone of each bone of a rigged mesh, default skeleton first): one range per mesh, returned when its last geometry is freed and reused by the following meshes; only the modified ranges are sent |
| `scene/sync_sets.rs` | Objects that the scene sync visits every frame, kept up to date by events (modified objects noted by `ObjectStore`, set of the objects that move by themselves and of what follows them, pending geometries, level-of-detail checks: still objects due when the camera has moved enough, objects placed by the GPU in turn (every 24 frames if the camera moves, every 0.4 s otherwise), every object every 10 s; worn HUDs listed on their changes and not with their wearer) instead of a walk over all the objects; the old LOD slice of all the objects stays behind `AURORA_SYNC_SLICE`; queue of the full syncs that did not fit in the frame's time budget, served from the nearest to the farthest (`Backlog`, `FullSyncBudget`); queue of the new objects, which wait for their first sync in arrival order without going through the frame's list (`Arrivals`, marked by the `ObjectStore` until that first sync) |
| `scene/sync_slow.rs` | `AURORA_SYNC_SLOW`: the steps, the full syncs and the slowest object of each frame whose scene sync is slower than a limit |
| `scene/mat_changes.rs` | Which objects a material change rebuilds: each material that arrives (or is given up) notes the material generation it changed at, and an object is rebuilt only when one of its own materials changed after its faces were built, instead of every object using materials at each change; the old way stays behind `AURORA_MATERIAL_REBUILD` |
| `scene/lod_watch.rs` | When to review the level of detail of a still object: each check leaves it the distance the camera can still travel before `lod_for` can change level (`lod_slack`, rounded down), and the object becomes due again when the camera has travelled it (queue sorted by deadline); a settings change (LOD factor, field of view) makes everything due. The check itself remains that of the sync: same levels at the same distances |
| `scene/sync_check.rs` | `AURORA_SYNC_CHECK` diagnostic: per second, the objects the sync visits per frame, according to what listed them and to their nature, and what came out of it (checked, re-placed, full sync) |
| `scene/still_check.rs` | `AURORA_STILL_CHECK` diagnostic: per second, what changed while the frame should be still (camera, our avatar, rebuilt faces and their reason, placed objects re-placed) |
| `scene/describe.rs` | The Dev Toolbox's "Debug to log / clipboard" (J1, J2), read-only: textures, lights and materials of the objects within 12 m, a summary line per attachment of the avatars within 10 m (faces drawn and left out), and for the avatars within 3 m or the target every face of every prim: texture entry as received, legacy or glTF alpha mode and cutoff, texture alpha class, pass, or why `rebuild_faces` left the face out; for L1 / L2, one prim face by face and each attachment's faces drawn and left out |
| `scene/sculpt_watch.rs` | Optional `AURORA_SCULPT_WATCH` diagnostic: changed attachment classification, geometry binding and sculpt-map streaming state for one avatar; no rendering or network behavior changes. |
| `scene/blend_order.rs` | Order of the frame's blended faces: those of rigged meshes first, back to front, drawn writing depth (`DrawLists::blend_depth`, the Legacy Viewer's rigged alpha pass), then the others back to front; a fixed order for faces at the same distance; `AURORA_RIGGED_BLEND` |
| `scene/sync_plan.rs` | Parallel placement (`frame_par.rs`) of the frame's objects, level by level of the parent chains: transform, LOD, bounds; update of the matrix alone or full sync |
| `frame_par.rs` | Parallel loops of the frame in which the calling thread takes part (placement of the sync, draw lists, poses, flexible prims): it starts the loop at once, a few workers of the rayon pool join it as they wake up, and those that arrive too late are not waited for. The number of helpers follows the amount of work (`Grain`); a short loop stays on the calling thread |
| `scene/anchors.rs` | What avatars wear, placed by the GPU: each bone palette ends with anchors, rigid world matrices written in each frame where their wearer moves or is posed (an impostor keeps its pose between pictures) — the first places the skeleton (skeleton space to world), the following ones the attachment points of the body as they are posed. The record of a worn prim contains its placement relative to the anchor of its point (`flags::ANCHORED`), that of a rigged mesh (worn mesh, bento, animesh worn or on the ground) and of an avatar's body reads the anchor of its skeleton: the vertex shaders compose. The GPU culling likewise composes the sphere of the worn prims (`CullObject::anchor`) and computes every frame that of the rigged meshes from the per-bone boxes of their geometry (`JointBoxes`, `object_flags::POSED`). The sync writes the anchors in use, sends them with the posed palettes, keeps the centre of the anchored objects up to date for the CPU (sorting of the transparent faces, hover, lights, impostors; the lists the CPU still builds test a rigged mesh with the extent of its skeleton) and no longer visits these objects as long as only their wearer moves. Left to the sync: HUDs, flexible prims, objects that move by themselves and what is linked to them |
| `scene/texture_cache.rs` | Disk cache of the transcoded textures (`cache/tex/<id>.bc7`): BC7 mip chain produced by a decoding task, alpha class, picking mask and header of the JPEG2000 image; read back at the wanted level without decoding when returning to a region, written by the `aurora-texcache` thread; a file with no levels records that a texture must stay in RGBA8 |
| `app/texture_refresh.rs` | "Refresh textures" of an object (every texture of its linkset), of an avatar (its bakes, then `avatartexturesrequest`) and "Force appearance update (rebake)" (`World::rebake`: IncrementCOFVersion then the bake request, `world/outfit.rs`). `TextureStreamer::refresh` forgets a texture once no read, decode or download of it is under way, removes its files in the disk cache and downloads it again; an upload decoded before is dropped (`Entry::epoch`) |
| `scene/tangents.rs` | Per-vertex tangents of the normal maps, built with the geometry by the background tasks as the Legacy Viewer builds them: Lengyel's for prims and sculpts (LLVolumeFace::createTangents), MikkTSpace for meshes (LLVolumeFace::cacheOptimize, in the mesh's upload proportions, `NormalizedScale`; vertices split where corners disagree, Lengyel's past 16-bit indices). Packed in one byte, the `w` of the vertex normal (`aurora_render::pack_tangent`: angle around the normal, handedness as the sign), so the vertex keeps its 24 bytes; `object.wgsl` unpacks them and `perturb_normal` uses them, the per-pixel frame where a face has none (`AURORA_TANGENTS`) |
| `scene/mesh_bytes.rs` | What is held of a mesh asset: the parts downloaded or read (its start, then the blocks drawn), each kept as it came and read from the part that holds a block, the disk cache file of those parts (`AMSH`, or a whole asset as it is), and the byte ranges still to download, joined when close |
| `scene/fetch_priority.rs` | Rank of every asset request in the fetcher's queue (`aurora-net/src/fetch.rs`): an urgent lane first (the sky, animations, sounds, materials, our own bakes and worn meshes, a window's image), then meshes and textures together by their size on screen, what shows nothing yet first; ranks sent again when the view changes them, requests dropped when nothing uses the asset any more |
| `scene/texture_load.rs` | Measurement of the texture loads: time of the decoding tasks per step (codec, conversion, RGBA, mips, staging memory) and `texture load:` line of the log at the end of each load (duration, decodes per level, bytes downloaded, disk cache) |
| `scene/texture_importance.rs` | Size a face's textures are loaded for, as the Legacy Viewer computes it (LLViewerTextureList::updateImageDecodePriority, LLFace::calcPixelArea, calcImportanceToCamera): the circle around the object from its nearest point, boosted up to eight times in area near the camera and in the middle of the view (none while the camera moves fast or under memory pressure), divided by the repeats squared; the same size for every map of the face. `AURORA_TEXTURE_SIZING=screen` brings the former sizing back |
| `scene/texture_budget.rs` | Texture memory budget: current target (the setting, less when the rest of what the viewer holds on the card leaves less room under the budget that the driver grants it, `aurora-render/src/vram.rs`), and pressure level when the pages exceed it. Under pressure, `scene/textures.rs` shrinks the textures loaded more finely than their on-screen size, off-screen ones first, by copying their levels on the GPU (`TextureTable::shrink`), without decoding them again; the pages they leave are compacted without waiting. Override: `AURORA_TEXTURE_BUDGET_MB` |
| `scene/geometry_need.rs` | Geometry each region needed at its last visit (peak use of the geometry arena while the agent was there, `cache/geometry-needs.txt`): behind a loading or teleport screen the arena grows at once to it, or ahead of need beyond three quarters full, so that its buffers are not replaced in the middle of the scene (`GeometryArena::grow_ahead`) |
| `aurora-render/src/frame_priority.rs` | Priority of the threads a frame waits for (main, render, frame helpers, texture pages): one step above normal, applied by each thread at its next turn; `AURORA_FRAME_PRIORITY=normal` or `alt` to compare |

`aurora-audio/src/processing.rs` owns Sonora/WebRTC AEC3, AGC2, noise suppression
and high-pass processing. Device callbacks only queue microphone samples and
bounded 10 ms stereo reference blocks tapped after the final mixer gains.
`aurora-audio/src/wind.rs` synthesises the ambient wind noise in the mixer
(LLWindGen: pink noise through a resonant low-pass, on the Ambient channel),
moving towards targets the viewer sets with `AudioEngine::set_wind`;
`scene/wind_sound.rs` computes them each frame from the region's wind at our
avatar and its motion, faded in while flying above the water.
The capture device worker processes 48 kHz mono blocks before manual input gain
and metering; preferences apply live without running DSP on the UI thread.
`ui/options/pages/sound.rs` owns Sound and voice preferences and the shared ear/noise
controls in the microphone popup. `AudioSettings` persists the processing,
camera/avatar listening and visualizer options with the Legacy Viewer defaults.
`scene/live_morphs.rs` implements the Legacy Viewer's 24 Hz Aah/Ooh babble
curves and holds the system body's other live morphs: the body physics' driven
parameters (`scene/body_physics.rs`), the facial expressions and the hand
poses (`scene/default_motions.rs`). Static deltas are shared, sparse, in a
reserved prefix of the GPU palette buffer: per vertex of a part, where its
entries start and how many there are; per entry, the position delta and
weight column of one morph, then its normal delta. The four matrices after
each skeleton's anchors hold one weight per live parameter (64 columns), and
from the last column one per body part, not zero while one of its weights is
not: a part at rest costs the vertex shader one read. The relaxed hands, the
usual pose, are baked into the shape (`scene/mod.rs`, `RELAXED_HANDS`), the
live hand weights moving from them.
Object, shadow and picking vertex passes apply the deltas before skinning,
including shape-morphed system parts, without rebuilding meshes per speech,
expression or physics frame.
`scene/default_motions.rs` ports LLEmote, LLHandMotion, LLBodyNoiseMotion and
LLBreatheMotionRot: the expressions are built-in motions without joints
(registered in the animation streamer, never fetched), started by the
simulator or by the `emote_name` of an animation's header (`anim::Controller::sync`);
the hand pose is the `hand_pose` of the strongest animation, blended in
0.2 s; breathing and body noise are joint states of the controller's
default motions, the noise blended additively.

The voice indicators of the classic name tags are drawn by
`ui/voice_tag.rs` (the name plates have their own outline and halo,
`ui/nameplate.rs`): outline smoothed according to the speech level, colours
calibrated on the WebRTC scale (red from 0.58, the local microphone peaking
around 0.63), struck-through Tabler microphone (`microphone-off`) for the
avatars outside voice. `voice.rs` guarantees the entry of our avatar from the
local microphone, even without an echo of its presence from the voice server.

The configurable Import shortcut and Build › Import both toggle
`imports/ui/launcher.rs`: seven type-specific actions, Settings and a plain
close cross in a horizontal toolstrip, without title, shortcut text, help
footer or common background. Each button uses `widgets::glass_action_icon`
for independent world blur, a translucent palette tint and the theme's
border, shadow and radius; opaque skins remain opaque. `widgets::toolstrip`
retains movable layout placement without painting a group surface. Its first measured opening uses an explicit top-midpoint default below the bars; saved placement wins. Other untouched windows initialize a valid point among all eight perimeter anchors and its offset from their unchanged default geometry; the same eight-point rule applies after completed moves. The
filtered file picker is scheduled when the chosen type has no listed file;
otherwise its inspector opens directly. Choosing the displayed type closes
its window. The chooser stays open above overlapping inspectors. Newly shown import
windows and Settings share the same top-left default; saved placement wins.
Settings contains only permissions and payment. Its five independent columns
per type cover next-owner modify/copy/transfer, group sharing and everyone-copy.
Missing sharing fields in saved rows inherit the legacy common flags; explicit
per-type overrides are saved without resetting other rows.
`imports/ui/inspector.rs` draws dedicated Texture, Sound, Animation,
Material, Mesh and Local floaters, each with its own saved geometry and a
filtered file list at the left. The right pane keeps the shared metadata,
the current asset panel. `imports/sound.rs` retains the original PCM, a cached
amplitude envelope and non-destructive crop / gain / segmentation options. `ui/sound_panel.rs`
draws the waveform and a playhead started when the audio engine receives the
prepared clip; gain commits restart audition with the processed PCM, while
cropping stops stale audio. Gain is applied once, with a digital ceiling and
waveform clipping indication; preview and upload use the same renderer.
PCM cropping, upload encoding and preview preparation stay on workers, and
each confirmed queue step freezes the sound options with its permissions.
Optional splitting plans exact consecutive sample ranges, suffixed inventory
names and per-piece costs. The parent draft stays until the last receipt;
acknowledged indices survive a failed draft's explicit restoration, so a retry
does not charge its successful pieces again. Bounded long WAV sources are
accepted for editing; each output still obeys the grid's asset duration limit.
Animations pair the preview with labelled metadata, loop units and easing
grids in a narrow inspector, with a full reserved reset-button row below the
preview. Dedicated materials use a narrow vertical arrangement with a larger
preview and unframed map rows in one column. Mesh and material palettes open
a separate small floater, so clicking the shared `widgets::color_button`
does not dismiss a parent menu. Colors update the preview's render uniforms
immediately, without rebuilding geometry or changing imported data.
Multiple keeps the complete inspector and its type tabs;
`Imports::inspector` keeps that window identity independent of its active
type tab. Parsing, preview jobs and the confirmed upload queue stay shared.
Collada skin controllers absent from visual-scene instances are decoded through
that same scene path, once per controller, following LLDAELoader::OpenFile /
processDomModel. Explicit instancing and nested transforms remain intact.
The mesh inspector allocates its panel once and keeps its metadata/preview,
tab viewport and bottom summary in independent bounded rectangles. Its section
viewport owns scrolling; the shared outer details scroller is bypassed for
upload meshes, so scroll bars cannot feed their dimensions back into the window.
Mesh completion follows LLMeshUploadThread::onCompleted and
on_new_single_inventory_upload_complete: a complete server acknowledgement
creates the object inventory entry with the granted permissions and flags.
An immediate FetchInventory2 re-read is not a condition of paid-upload success;
ordinary inventory synchronization can subsequently reconcile the entry.

`movement/movelock.rs` owns the optional native-sit Movelock state machine;
`app/movelock.rs` samples voluntary input and travel eligibility before the
movement controllers and dispatches ordinary SIT_ON_GROUND/STAND_UP controls.
The existing bridge lock remains the per-avatar default. Native mode requires
Aurora's integrated AO to be active with at least one compatible standing idle.
Otherwise its effective bridge preferences enable the existing scripted anchor;
the saved native choice is retained and resumes automatically when the AO is
ready. Routing follows AO eligibility even while moving or manually seated;
the bridge retains its normal movement/sitting rules. Only the scripted anchor
is disabled while native mode is eligible, retaining other bridge services.
Preparation and the owned ground sit are mapped to Standing by the AO;
manual seats take priority.
Preparation starts only after simulator travel and local visual settling fall
below the ordinary 0.5 m/s walk-stop threshold, including vertical flight.
The legacy interpolation residual and Aurora's drawn glide both count; key
release alone cannot force the idle animation during deceleration. Residual
motion cannot release an already owned anchor; voluntary input still can.
Native acquisition uses physical contact and the newest stock animation signal;
completed jump/landing transitions and a supported stale fall cannot suspend it
forever. Dry support near water is distinct from swimming. Water proximity and
jump/fall signals cannot release a confirmed ground sit; active water/travel
controllers, user input, region changes and AO compatibility still take priority.
Localized status and transition logs distinguish region arrival, water movement,
airborne transitions and other travel controllers.
The optional position-lock indicator starts disabled. When enabled, its area
can be dragged within the work area and its position uses the shared saved UI
layout; a simple click still opens the lock options. Compact state labels keep
the indicator short; its tooltip and the options retain the full diagnostic.
Acquisition waits at least 250 ms at rest, releases immediately for movement or mode
changes, bounds retries, handles cancelled requests' late echoes and drops
ownership across regions. Water movement, jumps/falls and autopilot pause it.
Click-to-walk/run releases an owned ground sit and keeps its autopilot alive
through stand-up acknowledgement; manual ground/chair seats still block it.
`ao/anchor.rs` caches per-asset channel coverage against the actual constrained
sit animation: every possible standing group/parallel-track cycle must loop
and override all its rotation/translation channels at a higher joint priority.
The AO broadcasts the ordinary standing selection while retaining the native
ground-sit system signal for the simulator and observers. A verified set is
retained through asynchronous stand-up when AO preferences change. No animation is
uploaded and no seat object is created. Simulator push resistance and observer
appearance require human grid acceptance.

`ao/anchor_blend.rs` keeps one fully weighted standing group through native
entry, stand cycles and movement handover. Acquisition additionally waits for
the simulator's animation echo, the authored ease-in and a 150 ms delivery
margin. The existing stand clock is retained through preparation/request/echo.
The regular AO starts the next group before stopping its full cover; a separate
raw-animation motion-controller regression reproduces the observer's pelvis dip
without the owner's pose filtering. Standing acknowledgements before walking
retain coverage too. Missing movement replacements have a bounded timeout;
manual seats, swimming and explicit AO disable release the handover. Ordinary
AO playback never queries these easing timings. The offline animation mock
echoes batched starts/stops and preserves separate custom motions across native
system-state changes. There is no bridge script change or new render pass.

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
single selected preview above a virtual folder list. Inventory, Local and
Aurora Library tabs preserve folder browsing; the latter scopes the real
#Aurora/Textures Library subtree. Rows paint resource icons without requesting
thumbnail assets. Default/Blank/Transparent/None appear below the list, with
inapplicable choices disabled; a validated UUID sits directly below them.
The collapsible "Local files" folder belongs to Local and filters textures /
materials separately. Inventory roots are opened once and can stay collapsed. `app/imports.rs` prepares the local images
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

`inventory/ais.rs` reads agent folders and items through InventoryAPIv3,
following `AISAPI::FetchCategoryChildren` / `FetchItem`. Complete folder
snapshots include every child's favorite metadata, links and embedded
originals, with bounded parallel HTTP reads. Favorite mutations verify the
whole parent through AIS; overlaying only the last changed flag onto a
legacy listing would erase earlier favorites in the same folder. Library
and servers without AIS retain their legacy fetch paths. The model tracks
whether favorite metadata is authoritative: partial/UDP/legacy records
preserve known flags, while complete AIS records can explicitly clear them.
Cache version 3 invalidates older snapshots once, reloading favorites that
earlier builds may have lost from their local listing.
Readback retries never repeat a write, and diagnostics log only field states.
Inventory links read and modify their original's favorite through
`world/inventory/actions.rs`; the menu, filter, count and star share that
rule. Multi-selection sets a common state and deduplicates aliases,
following `set_favorite` / `doToSelected` (llinventoryfunctions.cpp).

`session/inventory_updates.rs` receives `UpdateCreateInventoryItem` and
`BulkUpdateInventory` directly over HTTP, like Firestorm's `LLEventPollImpl`,
and the LLSD variants of `LargeGenericMessage` / `GenericMessage`, in
addition to the UDP transport. `InventoryUpdated` applies items and folders
as soon as they arrive. `world/inventory.rs` advances the versions of the
loaded parents, ignores older replies and avoids counting repeated records
again.

Confirmed attachment removal (`ObjectsKilled` or an unparenting update)
unregisters our root's `AttachItemID` before discarding its object state.
`world/appearance.rs::unlink_attachment` removes only its Current Outfit
links locally and queues their individual AIS deletions through the existing
background inventory edits, without another appearance synchronization.
The original and saved outfits remain available; teleport/crossing kill
protection runs before unregistering.

`ui/inventory/progress.rs` distinguishes the submission of a mutation from
the tracking of the copies in the background: several copies can be tracked
without disabling the other commands. `InventoryProgress` passes on steps
and counters; the status line, shared between windows, shows the activity
and the elapsed time, then the end confirmation. Errors keep the possibility
of reloading the folders concerned, without replaying the copies or the
moves.

### Window surfaces

`ui/widgets.rs::window_dragging` identifies a move or resize of any floater or
bare window from its active move ID or the grips of `ui/widgets/resize.rs`.
The shared `ui/menu.rs` entry points dismiss context menus and their popup
state during that gesture, regardless of their owner or drawing order.
`ui/context.rs` applies the same rule to the world pie. Native main-window
move/resize events also clear popup and world-menu state. Click tolerance
still distinguishes a deliberate move from pointer jitter or a child control.

`world/radar.rs` holds the radar of the People window (sweep of the avatars once per second, first
appearance, crossings reported, regions left behind by a teleport), `ui/people/radar.rs`
its table (Firestorm's look), its rows, its settings and its options menu; `ui/people/nearby.rs`
the same list in Aurora's look (rows grouped by chat range, compact density, sort pills, quick
actions over the row under the pointer), `ui/people/sheet.rs` the sheet of the person clicked,
at the right (rights of a friendship, what the radar knows, the avatar menu under "…"; the window
widens for it before its frame and gives the width back, "Nickname and contact sets"),
`ui/people/friends.rs`, `groups.rs` (with a group's sheet), `sets.rs`, `recent.rs` and
`blocked.rs` the other tabs of Aurora's look (the Contacts tab's rights, actions and dialogs,
reused from `ui/contacts.rs`), `ui/people/rows.rs` what their rows share (faces, names, a right
end that never covers anything, quick actions, Ctrl / Shift selection), `world/recent.rs` the
residents met lately (LLRecentPeople), `demo_people.rs` their offline scenario; `ui/people.rs`
the window, its tabs, and the simple Friends, Groups and Blocked lists of Firestorm's look.
`ui/widgets/pills.rs` shares the tabs in pills, the filter pills and the search box of the
redrawn windows (group profile, About Land, People).
The "Invisible" status (right to see us online withdrawn from each friend, then given back to those who had it)
is in `world/status.rs`.
`ui/widgets/unread.rs` is the one unread counter (islands, dock, Firestorm's
bars, Conversations list): dark red, its digits' ink centred to the pixel.
Text beside an icon, a badge or in a box is placed where epaint draws it
(`widgets::ink_rect`, `galley_baseline`, `row_text_origin`: each glyph and
the face's vertical shift rounded to the pixel); `ui/widgets/alignment_tests.rs`
checks the shared widgets with every embedded font and a system font at
several scales.
A list over a footer (buttons, a note) goes through `widgets::above_footer`:
the list gets exactly the room the footer measured, so the window never
grows from frame to frame; `ui/widgets/steady_tests.rs`
(`assert_window_steady`) runs a window's real `show` over frames, zooms and
a larger text and checks that its rect stays put with its bottom button
inside.
`ui/widgets/keycap.rs` draws the keys of a shortcut (list of
shortcuts, microphone panel). `ui/widgets/color.rs` is the compact colour picker (`color_button`,
`color_panel`, RGB / HSV / hexadecimal conversions) and the eyedropper it shares
with the application (`app/color_picker.rs`).
`ui/widgets/workspace.rs` shares the navigation, the detail panels, the
dialogs and the rows of the user inventory and of object contents
(density, hover, selection and name style). `build/ui/compact.rs` arranges
building in seven sections in a narrow inspector: General contains identity, group/share/deed, sale and search, while Rights keeps everyone/next-owner permissions. The five tools share the navigation rail; titles follow the section/tool, and edit selection/actions stay at the bottom of the main panel. The inspector resizes on both axes, fills the chosen height and retains its geometry across sections, tools and folding. The object inventory fills the remaining section viewport; tool hint space uses the exact wrapped text galley, preventing frame-by-frame window growth. Each transform vector has its own caption-line copy/paste controls; linked editing, face selection, both-sided stretching, root-axis editing and reflection probe selection share the footer. Probe selection is available without an existing selection, defaults off at startup and filters click/menu rays, face selection, cursor-wheel stacks and selection rectangles. Shape rows (including sculpted prims) and the material map/settings/UV grids reuse the existing editing commands. `build/ui/compact_tests.rs` covers the permission split, shared selection options, per-vector clipboard interactions in all manipulators, and actual pointer resizing with section/tool/fold transitions, stable narrow-tool geometry over repeated frames, and inventory filling/filtering after height changes.
The footer also exposes World / Local / Reference through three persistent
icon buttons in a vertical column below an XYZ symbol, with a violet face for
the active mode and muted inactive icons. Rez-under-land-group shares this
footer in Edit and Create. Linked-prim and face navigation have separate arrows.
The shared Floater supports a compact status row in its title; construction
uses independent lock, no-mod and ownership states. Material projection/options
are inline rather than collapsed.
Construction uses the shared Floater's steady body sizing: content cannot widen
the chosen inspector, including during or after PBR numeric scrubbing. PBR rows
reserve the measured number field and its step buttons before truncating labels,
so enlarged fonts fit without feeding minimum-width growth back into the window.
The shared footer keeps its own bounded viewport so wider section content cannot
push its coordinate buttons outside the inspector.
`build::can_transform` applies effective simulator
Move / Modify flags to numeric controls and manipulators before any local echo,
with whole-selection validation and checks during dragging and release.
`build::family` follows `LLViewerObject::addThisAndNonJointChildren`: seated
avatars and their attachments are outside the editable linkset. Selection
bounds, highlights, packets and linked-root transform compensation share this
boundary. Object menus inspect each prim's direct avatar children separately
to keep attachment of occupied objects disabled.
Blinn-Phong UV controls remain available for empty normal/specular channels;
non-default map transforms remain in RenderMaterials even before assigning maps.
Floating-point spinners in `build/ui/common.rs` retain the active drag value
until release, so panels that submit only committed edits do not reset it each
frame. Drafts are used only while their original widget is dragging or releasing;
numeric transform previews still report their final commit on release.
Typed float/integer build values commit on Enter or focus loss, with Escape
canceling the draft; text-only changes never submit unchanged object values.
The texture panel retains a submitted Blinn-Phong material until the simulator's
replacement material is loaded, keeping map controls and their input focus stable
between ObjectUpdate and RenderMaterials. Loaded replacements and explicit
material removal supersede the pending edit.
Float/integer build fields and color fields share themed increment/decrement
buttons; steps respect bounds and disabled permissions and commit once per click.
After confirmed simulator parent updates, whole-object selection and highlighting
resolve and deduplicate actual roots, preventing old roots from keeping their
root colors or duplicating outlines. Link requests report pending until all
parents agree; only then is the outgoing ObjectLink order retained, independent
of UDP arrival order. Existing linksets without a locally confirmed instruction
still require simulator-side link-number verification.
`build/ui/contents_selection.rs` owns the task list's Ctrl/Shift selection,
focus and Delete/F2 dispatch before global object shortcuts. Rename requests
focus once and commits on Enter, blur or leaving the panel, rechecking access
to the pinned object/item; Escape cancels. Filtering prunes hidden selections.
`build/ui/contents_keyboard_tests.rs` drives the native inspector with pointer
and key events, checking that task Delete never sends an object DeRez.
`ui/inventory/properties.rs` supplies the same properties floater and panel
for agent items and task items. `build/ui/item_properties.rs` pins the source
object/item and rechecks access before saving through UpdateTaskInventory;
task thumbnails remain unavailable because the task protocol has no thumbnail
field. `build/ui/contents_permissions.rs` provides the eleven bulk type
filters, three permission columns and OK/Apply/Cancel. Its request job pins
the selected objects/options, fetches each inventory, skips failed, expired or
no-longer-modifiable sources, and continues after OK closes the floater.
`build/ui/contents_tests.rs` exercises task saves and the bulk UI/request path.
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
destination beacon) is never hidden. Its column is clipped in homogeneous
coordinates so its visible part remains when the summit is off screen
(`ui/minimap.rs`). The pin, direction arrow and location label return hit
rectangles tied to that destination; `app/click_pick.rs` consumes their
unmodified left press before requesting a world pick, after the UI and worn
HUDs. A name tag stands half a metre in front
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

Linden trees and grass (`LL_PCODE_LEGACY_TREE`, `LL_PCODE_LEGACY_GRASS`)
get their geometry from `aurora-prim` (`tree.rs`, `grass.rs`, ports of
`LLVOTree` and `LLVOGrass`) and are drawn like the other objects
(`scene/plants.rs`): a tree's mesh depends only on its species and detail
level and is shared by every tree of it, built by a background task and
placed with a uniform scale; a grass tuft has its own small mesh, built at
once from the terrain heights under its blades and rewritten in place when
terrain patches arrive for its region (`Region::terrain_revision`). Both
take their species texture whatever their texture entry says, alpha-tested
at 0.5, so they cast shadows and are picked through their cut-out.

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
screenshot (same frame captured as before, read just after). A photo is
such a frame (`Capture::Snapshot`): the render thread gives its targets the
photo's size for that frame alone, draws into an offscreen texture that is
not presented, reads it back and restores the window's targets
(`Backend::begin_snapshot` / `end_snapshot`). Beyond the card's largest
texture or the memory budget, the photo is drawn in tiles, each with an
off-centre projection, and stitched on the main thread. The depth under
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
of them. The camera (Alt-click, Object view and Focus) uses the same posed
geometry with a visible-texture alpha test; touch/build keeps invisible
Blend surfaces. CPU searches never intersect the unposed rest geometry of
GPU-posed meshes. Nothing is drawn or allocated per frame outside a click.
Name-tag hit planes and their rectangles are captured with the press and
tested against that frame's displayed depth, matching the label shader;
invisible touch geometry cannot steal the name plate's action.
`app/world_click_demo.rs` exercises these paths and other residents' scripted
attachments through actual queued presses/releases (`AURORA_DEMO_WORLD_CLICKS`).
The old read, which waited for the frame in progress then for everything the
GPU had queued (`Renderer::pick_world`), is now only used for comparison
(`AURORA_CLICK_PICK=wait` or `check`). If the render thread disappears
(panic), every wait ends, the pending clicks receive an empty reply and the
viewer closes cleanly.

Worn HUDs go through `scene/hud.rs` (lists of our avatar, interest of the textures, triangle picking) and the shared projection `aurora-render/src/hud.rs`. The attachment points in `Scene::object_transform` are relative to the screen, with the aspect of `mScreen`, without the position or the orientation of the avatar. The renderer draws the opaque / masked then the transparent faces after post-processing, in an independent depth, before egui; it reuses the geometry, the bindless textures, the materials and the UV animations of the world. `demo_hud.rs` provides the offline updates and touch replies.

The editing of HUDs reuses the building tools: `build/geom.rs` adapts their camera to the orthographic HUD projection (parallel rays, handles of constant size in pixels), `build/mod.rs::edit_parent` provides the transform of the attachment point to convert the moves into local coordinates. HUD and world selections stay separate. `build/manip.rs` sends the local positions on release with the linkset flag; the Object tab also uses the local values for HUDs. Direct moving with the chosen key (Ctrl by default) shares this same edit_parent and the picking of the visible faces, with the double-sided exceptions. Preferences › Shortcuts › Other › HUDs uses `ui/options/pages/shortcuts.rs` and the `keybinds::Action::HudDrag` action: the two keys share the capture, the clearing, the conflicts and the reset of the other shortcuts. Modifiers alone are captured on release and their left / right sides are equivalent. `Settings::sanitized` migrates the old `hud_drag_key` to the main key without overwriting the two choices already saved; `app/hud_drag.rs` uses these shortcuts for the gesture and the cursor. Settings version 10 replaces the former Alt-only default with Ctrl once (#266), while preserving other saved bindings and cleared slots.

What our avatar wears on its body is edited in the same way: `build/mod.rs` distinguishes three selection spaces (`SelectType`: world, worn object, HUD, never mixed, like `LLSelectMgr::canSelectObject`), `edit_parent` gives the reference frame of the attachment point as it is posed at that instant (animated bone included), and the positions typed or dragged are sent in that reference frame. With the "World" grid, the arrows follow that reference frame (`LLSelectMgr::getGrid`). A worn object cannot be deleted, taken, copied or linked and does not enter a selection rectangle; those of the other avatars stay unselectable. "Touch" and "Edit" in the menus of the inventory (`ui/inventory/context.rs`) and of the outfit (`ui/appearance/items.rs`) go through `CtxAction::TouchWorn` / `EditWorn`, resolved by `World::worn_attachment`. `demo_worn.rs` and `app/worn_demo.rs` provide the worn object and the offline scenario (`AURORA_DEMO_WORN`).

"Inspect Objects" (`ui/inspect.rs`, port of LLFloaterInspect with the Legacy Viewer's additions) lists the prims of the clicked object, or of everything an avatar wears: `app/inspect.rs` requests their properties through ObjectSelect / ObjectDeselect (without touching the selection of the building tools), assembles the rows every frame and outlines the prims in the world; `scene/inspect.rs` gives faces, vertices and triangles at the displayed level of detail, the textures and their memory, and the complexity of the linkset with the rules of `scene/complexity.rs`. The chosen columns are saved (`Settings::inspect_columns`, FSInspectColumnConfig).

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
viewer. The native stacks (exception and Rust panic) are written as
`module+0xoffset` with each module's PDB identity, read from the PE headers
in memory: the only form that can be symbolized after the process is gone.

`summary.rs` builds the public text of a crash report (`public-report.md` in
the session folder, written before the window opens): only values of a known
shape, re-validated from the session files, and the closed log vocabulary of
`privacy.rs`. The raw traces, panic messages, paths and the log stay local.

`window.rs` reuses Fluent, the Palette, the fonts, the Tabler icons
and the buttons of the viewer. `software.rs` draws its egui meshes on the
CPU and `softbuffer` presents the pixels, without initialising Vulkan or any
graphics adapter. The loop waits for events between frames.

"Send a crash report" opens GitHub's issue chooser in the browser, like
"Report a bug" in the viewer. While the built-in form is disabled (see
`bug_report/` above), `report.rs` stays out of the build. Once enabled, it
composes the draft from this snapshot and the markers of
the stopped session, and the button opens the same
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
| Cache or conversation history in a folder chosen in the preferences | the cache in `<chosen folder>\AuroraViewerCache\` (from the next start); the history in `<chosen folder>\<account uuid>\chat_logs\<grid>\` |
| Remembered password (hash, never the password) | Windows Credential Manager, "Aurora Viewer" service |

The explicit offline `AURORA_DEMO_AO=design-*` study populates `AoUi::prototype` only in demo mode. `ui/ao/prototype.rs` reuses the native AO body and shared widgets, with current/tabs/collapsible-section layouts and transient motor settings. `ao/demo.rs` seeds inventory links for the contributed swim pack; preview states go through the actual AO runtime, with stable animation signals and no motor/persistent movement changes. The maintainer selected the tab layout: normal sessions now embed `ui/movement.rs` in the AO manager, with the same inventory animation editor in the Animations tab. The earlier workbench remains an explicit offline comparison fixture only.

The integrated Movement tab persists physical settings independently of AO activation. Water instructions are short gesture rows using the current Up/Down primary and secondary bindings, with modifiers and mouse buttons drawn through shared keycaps. Page Up/Down labels use compact Page arrows. Unbound actions are explicit; the double-tap and jump-then-descend sequences are distinct. Detailed dive timing stays in a dynamically keyed tooltip. Remove the former swimming-animation navigation button and dense AO implementation paragraph; the ordinary Animations tab remains the editor. `movement::Runtime::swim_signal` supplies a stable swim state to `ao::runtime`, which alone chooses and broadcasts swimming animations through the normal AO lists/cycles. The AO never uploads or automatically seeds the contributed swim pack. Its former import button/uploader and legacy swim-seeding module have been removed; existing inventory swimming choices remain user-owned. Obstacle passage is separately enabled and uses Ctrl plus the current jump bindings, with explicit conflicting bindings winning and nonce-stable physical commands. The native key handler suppresses only the ordinary jump for that active passage chord. The shared toolbar reflects any enabled movement aid.

Flight settings use a neutral x1..x5 multiplier. Preferences migrate the legacy absolute `fly_speed` field and no longer serialize it; native x1 releases mode 2, while boosts convert to m/s before using the existing bridge wire contract. Horizontal/ascent and downward nominal speeds are separate. The flight payload validator admits the x5 descent target, with the same collision-aware LSL motor and unchanged run/Movelock behavior.

`movement/swim.rs` interprets the resolved ascent/descent actions: a short descent starts a complete dive; a short underwater ascent returns to the configured flotation height. At the surface, an ascent tap takes off unless an actual steep obstruction calls for a physical shore jump. A 0.5 s hold takes off and hands continued ascent to native/enhanced flight. Bounded bridge 1.13 modes 7/8/10 carry water-relative vertical goals; mode 9 carries a nonce-stable shore displacement with a rise-then-advance trajectory. `scene/picking.rs::swim_landing` uses the existing parallel picking index to find a nearby walkable top with headroom; terrain probes supply terrain landings. The simulator remains authoritative for collisions and positions. Pending water goals expire if unreachable, and disable/focus/region/bridge-loss paths release them. During an explicit bounded water destination, `syncMotionLock` temporarily yields the shared motor; normal `setLock`/`syncLock` and the accepted run regulator remain unchanged, and the user's Movelock activation stays on. AO swimming states still have one owner in the ordinary AO runtime; no effects/sounds are added.

Bridge 1.16 mode 5 implements the maintainer's remote-Movelock-anchor experiment. The existing bounded far-side destination becomes a temporary anchor. The motor uses exactly the accepted setLock(TRUE) parameters: VEHICLE_TYPE_SLED, linear/angular friction timescales 0.05, and llMoveToTarget(target, 0.05), refreshed while the action is live. It does not change gEnabled, gLocked, gAnchor, the user's Movelock selection or native key forwarding. It replaces the 1.15 impulse/near-target damping combination after human feedback showed 0.60 m progress, 1.31 m remaining and 11 unsuccessful impulse requests. No impulse, velocity assignment, teleport or avatar-position write is used. Replay retains the original target and 1.2 s server deadline; the viewer still allows 1.5 s for delivery. Arrival/target-plane crossing, cancellation, sitting, region change and timeout release the temporary target and restore VEHICLE_TYPE_NONE. If normal Movelock has already taken ownership, releasePass leaves its target/vehicle parameters intact. The normal setLock/syncLock functions and accepted run regulator remain unchanged. This motor remains subject to simulator collisions; a distant anchor does not guarantee penetration.

The private acknowledgement appends an optional bounded PassageReport: action, execution/result state, observed forward displacement, remaining distance and highest sampled forward velocity. It retains the v1.15 impulse-counter wire slot as zero; the anchor feedback no longer displays an impulse count. No coordinates or avatar/object identities are included. Final release emits the existing hidden OwnerSay status; a late startup HTTP reply cannot replace that terminal result. movement/passage.rs matches results to the current action, distinguishes startup/arrival/expiry/cancellation/Movelock ownership and reports a missing final result after five seconds. Feedback appears in AO Movement and never changes activation or claims crossing from command receipt alone. The offline passage-* fixtures are synthetic UI/protocol states, not physics validation.

Passage completion uses a 0.10 m radius, not the former 0.35 m: with the viewer's original 0.55 m far-side clearance, the latter could stop/brake with only 0.20 m clearance, inside its conservative 0.35 m avatar radius. Plane crossing still handles overshoot. A delayed deadline callback checks the observed arrival before classifying expiry as failure. Neither change grants collision bypass.

Bridge 1.17 replaces the unbounded two-second discovery announcement with viewer-initiated discovery. Aurora sends an object-addressed auroraHello request on a fixed negative channel through ScriptDialogReply (agent ObjectID, index zero), following Firestorm really_send_chat_from_viewer in fsnearbychathub.cpp. ChatFromViewer is rejected on negative channels; the initial 1.17 viewer used that invalid transport and the old offline simulator incorrectly accepted it. Discovery now runs only during startup/handshake or recovery from an HTTP error, at most once every five seconds. The worn root listens only to its owner and exact root UUID, checks those conditions again and throttles responses independently. A request arriving before permission/HTTP-in readiness is fulfilled once when startup completes. A previously paired bridge announces a replaced simulator endpoint once; unsupported viewers do not trigger indefinite announcements. URL acquisition retries stay silent unless discovery is pending. No endpoint, authentication key, physical command or public chat is sent in the probe; authenticated increasing-sequence HTTPS commands remain unchanged. Discovery never changes normal Movelock or motion flags and never detaches on viewer mismatch. Both the offline demo and factory tests now discover through actual outgoing probe commands instead of injecting periodic hellos. The 1.16 far-side anchor motor is unchanged in 1.17. Early failure reports preceded the discovery transport repair; the maintainer subsequently confirmed successful physical passage with the corrected viewer. Local control-flow checks do not simulate Havok; actual collision behavior still requires human validation.

Bridge diagnostics retain the actual provisioning stage or command failure in AO Movement and passage refusal text. aurora-net classifies timeouts, connection failures, HTTP status, malformed acknowledgement and sequence mismatch without formatting reqwest errors, their source chain, private URLs or response bodies. The viewer retains this safe message instead of replacing it with a generic unavailable label. Existing five-second discovery/HTTP recovery and motor watchdog remain in place; errors do not change the user's Movelock selection or detach the bridge. Offline tests explicitly reject ChatFromViewer discovery and wrong dialog addresses, exercise request loss/recovery, and verify cause propagation, parsing distinctions and error privacy. No LSL source or physical motor changes are part of this repair.

After the corrected discovery transport, the maintainer confirmed that script 1.17 physically crosses a wall. Passage selection now considers the nearest solid front face up to 3 m from the avatar centre (previously 1.5 m), while keeping the same far-side box clearance and conservative free-space rejection. The longer approach does not move the exit farther beyond the wall; total travel remains bounded at 8 m and the validated motor/LSL source is unchanged. Swimming's obstacle/shore probes retain their own reach.

Bridge 1.18 makes the temporary passage anchor slightly firmer by changing only its llMoveToTarget tau from 0.050 s to 0.045 s, above the documented minimum of two physics frames (2/45 s). This is a response-time adjustment, not a claimed fixed percentage gain in simulator force. The SLED/friction parameters, target, completion checks, replay guard, 1.2 s action deadline and cleanup/Movelock ownership remain unchanged. Normal Movelock still uses its accepted 0.050 s response. No impulse, new persistent force or coordinate assignment is introduced. The viewer keeps the 3 m obstacle reach and 8 m total travel limit; the stable inventory object/script names allow the factory to upgrade the same bridge.

Passage selection now adds 0.60 m beyond the inflated far-side box instead of 0.15 m: the goal moves 0.45 m farther along the passage direction, giving 1.00 m clearance for a perpendicular wall (including the 0.40 m box inflation). This is independent of the avatar's approach distance. The existing free-space and region checks validate the farther destination, and total travel remains bounded at 8 m. Detection still reaches 3 m. The compatible 1.18 script, its 0.045 s passage response, 0.10 m completion radius and 1.2 s deadline are unchanged; only its clearance comment is made version-independent.

Water entry and an underwater ascent tap dispatch the same Surface action, selecting bounded mode 8 and the ordinary AO SwimmingUp state. A fall in progress starts the return at entry; descending through flotation is not arrival. Completion accepts the height-aware target within 0.18 m and vertical velocity between -0.5 and 1.0 m/s. Depth hysteresis uses that same target: a surface swimmer submerges below it by 0.42 m; a submerged swimmer returns within 0.18 m. The settled previous float offset still protects live height changes. The shared physical root drives classification, so local visual waves cannot toggle AO states. An underwater ascent press remains a return throughout its hold/release, even if it arrives before release; another ascent press continues the destination. Descent cancels it and retains deliberate depth. A fresh surface tap jumps in open water or uses the existing collision-aware shore landing; a fresh 0.5-second surface hold requests flight when permitted. Surface up input alone no longer selects an underwater ascent animation. Observed distance changes of at least 0.05 m renew the surface budget; an unchanged blocked destination still expires. Disable, sitting, suspension, bridge loss, shallow/no water and passage release the destination. Entry assistance respects normal Movelock ownership. Bridge 1.23 bounds ascent to 3-5 m/s, ramps its command at 6 m/s per second, and brakes with a gain of 4 near the goal; its combined horizontal/vertical target remains bounded to 1.6 m. Existing dive, flight, run, lock and passage motors are unchanged. The Legacy AO's checkBelowWater only maps animations; these physical controls are Aurora extensions requested by the maintainer.

Bridge 1.19 softens only the shared mode 8 surface-return motor. Horizontal swim settings and the existing 0.2 s steering response/target envelope remain intact. A separate signed vertical command is capped at min(selected swim speed, 2 m/s), changes by at most 1.5 m/s per second of bounded script tick time and approaches error * 1.2 near flotation. The vertical target is clamped between the current height and flotation, including an overshoot while the command ramp reverses. These are command/target bounds, not measured Havok speed or acceleration guarantees. Release, normal Movelock ownership and mode changes clear the ramp; same-mode command refreshes retain it. Dive, takeoff, ordinary surface/depth swimming, run, passage and normal Movelock parameters are unchanged. The native surface completion, progress budget and vertical interruption rules remain unchanged. The compatible 1.19 source updates through the existing bridge factory with stable inventory names.

Movement enhancements remain experimental at the maintainer's request. The integrated AO Movement tab and panel heading explicitly say EXPERIMENTAL; the shared movement-title localization also marks its menu, toolbar and keybinding names. English/French labels identify the physical enhancements without marking ordinary AO animation management as experimental. No physical controller or radar behavior changes accompany the label.


Bridge 1.20 adds authenticated read-only services without changing the accepted physical motors. `aurora-net/src/bridge/query.rs` owns bounded altitude/script-statistics requests and strict JSON/base64 decoding, with the same simulator HTTPS trust/hostname rules as motor requests, no redirects, a three-second timeout and a 4 KiB response cap. The LSL query dispatcher authenticates and limits the request before returning independently of motor sequence, watchdog, permissions and timers; there are no new global motor variables. Names are base64 text so numeric/JSON-shaped object names cannot change the response schema. Neither payloads nor endpoint credentials are logged.

`world/bridge/services.rs` schedules the optional radar corrections and explicit object inspections; its `services/tests.rs` covers bounds, expiry, authority and session changes. This follows Firestorm FSRadar::updateRadarList/getZOffsets and handle_script_info/getScriptInfo (fsradar.cpp, llviewermenu.cpp and fs_resources/EBEDD1D2-*.lsltxt, originally LGPL 2.1). Only missing coarse altitudes are queried in batches of at most 32 (a smaller request than Firestorm's 60 to bound script/HTTP work), no more than one batch per second and one attempt per avatar every seven seconds. Unqueried avatars and the oldest attempts take priority so a crowd larger than seven batches cannot starve later entries. Corrections expire after fourteen seconds and belong to the avatar's sighting region and the current agent/region/bridge-token epoch. Known native/coarse heights always win over cached or delayed bridge replies; unknown heights stay unknown without a valid reply. The simulator may resolve avatars in its documented adjacent-region border zone; ordinary object inspection is restricted to the bridge's current region. Radar correction defaults on, with an opt-out under People / Nearby / radar options; arrival/departure alerts keep their existing native behavior and defaults.

Object / Script information resolves the selected linkset root, queues while the existing bridge provisions, then prints only a localized local-chat report: running/total scripts, allowed memory in KiB, average CPU in milliseconds and nonzero pathfinding time. It does not read script source or claim to measure live heap usage. The latest explicit inspection supersedes the previous one, and late replies after repair, teleport or relog are discarded. A current read failure requests throttled quiet endpoint rediscovery without resetting the motor sequence, pending acknowledgement, enabled selection or physical state; a repeated hello for the same endpoint remains idempotent. Inspection and radar reads have separate request UUIDs and do not keep movement alive. Offline fixtures use the actual factory/typed command path with synthetic replies, never a grid.


Radar cleanup (2026-10-16): the reported 2500 m target was actually at 250 m; its approximately 235 m range from an observer at 20 m was valid. The maintainer requested removing the speculative additions. Known positions are authoritative as in Firestorm: no verification queries, pose-age heuristics, per-region coarse clocks, raw-height fingerprints or diagnostic counters remain. The simple radar option and its default-on behavior remain. `world/radar.rs` still resolves the complete bounded parent chain so missing/cyclic seat ancestors use coarse fallback instead of seat-relative region coordinates (matching LLWorld::getAvatars' orphan exclusion). Nil targets are filtered before batching. The read-only protocol, LSL 1.20 source, object inspection and accepted physical motors are unchanged.

AO inventory startup ownership (2026-10-16): `Catalog::refresh` reads `#Aurora/Aurora AO`; an existing folder, even empty, prevents fallback. `ao/bootstrap.rs` reads the fully fetched Legacy Viewer catalogue only when that folder is absent, waits for unresolved original items, and reconstructs recognized sets, options, cycles, groups, tracks and ordered animation links without changing the source or uploading assets. It waits for the independent bridge factory to acknowledge the shared `#Aurora` parent, avoiding competing startup parent creation. Reconstruction stages an `Aurora AO (Legacy Viewer import)` folder and renames it to `Aurora AO` only after its preceding AIS operations succeed. Interrupted staging is retained for inspection and never blindly replayed; failures remain local AO status, with Reload resetting the source fetch state. Missing/empty sources produce no AO mutation. Saved valid Aurora selection is preserved; fallback selection is remapped to its newly created set. Bridge creation/version checks already run independently of AO and Movelock activation; the physical/controller source is unchanged.


AO swimming ownership (2026-10-16): Aurora never writes to #Firestorm or any descendant during startup, successful reconstruction, interrupted reconstruction or AO edits. Original animation targets may be read and linked under #Aurora/Aurora AO; the AO mutation guard rejects source writes, removals, renames and moves into/out of that tree. The bridge factory issues only a reliable detach for an exactly identified worn Legacy Viewer bridge (owned object, versioned inventory name, source ancestry and Center 2 root), once per worn root; no script or inventory edit, authentication or reattachment of that legacy bridge. The four actual Legacy Viewer swim states (Floating/Swim.H, Swimming Forward/Swim.N, Swimming Up/Swim.U and Swimming Down/Swim.D) are fetched and reconstructed with their original animation links and cycle settings. The source reader ignores Aurora-only folders. Before writing a new migrated set, `ao/completion.rs` fills missing destination water states from the approved real donor configurations: underwater float from surface float, underwater swimming from surface swimming, flight exit from Flying Up, obstacle exit from Jumping, dive jump from Flying, dive entry from Swimming Down, Cannonball from Crouching and cushioned entry from Falling. It preserves existing assigned slots, main lists, simultaneous groups and independent track cycles. Missing/unresolved donors remain missing; no bundled clip or uploaded asset is manufactured.

Each Aurora set requires eight core swimming states with at least one resolved inventory animation in each: surface idle, swimming, down, up, underwater idle, flight exit, water entry and obstacle exit; four optional extensions are described below. The native Animations tab exposes their inventory pickers, link replacement/removal and drag/drop. Resolved assets in all eight slots of the selected, fetched set are required before swimming can be enabled; incomplete selections disable saved swimming, and loading/failure suppresses the effective motor without guessing readiness. No older Bridge-folder stroke discovery supplies missing slots. Active swimming owns its animations through the existing AO runtime and AgentAnimation even while ordinary AO overrides are off/paused. Surface and underwater idle signals are distinct despite sharing the stock hover UUID. Water entry remains selected only during the initial downward fall while buoyancy cushions it, then yields to the existing return-to-surface animation. Flight exit uses the near-surface part of the physical flight-exit action; deeper ascent keeps the up animation. Obstacle exits follow the separate bounded shore-jump state. Cancellation, disable, bridge loss, sitting and suspension release the contexts. Completed non-looping transition clips are not replayed every frame; a new context entry rearms them. These core slots retain the established physical swim motors; passive jump observers are described below.

The explicit offline AO design preview carries the chosen swimming context alongside its stock signal UUID, so surface/underwater idle and jump/fall exits cannot alias ordinary hover/jump/fall states. This path is restricted to the synthetic demo and has no grid commands.

AO startup service readiness (2026-10-16): cached source data can be complete before the login seed supplies InventoryAPIv3 and FetchInventoryDescendents2. Bootstrap now waits for both nonempty services in the current region, completed arrival, no teleport and a released inventory-cache hold before consuming its one reconstruction attempt. Read-only source fetching continues while waiting; the existing AO loading label includes this bootstrap state. A native AO arriving during the wait still wins, including an empty one. This pre-send gate does not retry an ambiguous failed AIS mutation or alter interrupted-staging protection. The explicit firestorm-wait demo delays synthetic services until frame 720 and exercises the production gate.

Explicit AO migration (2026-10-16): `AoUi::migration` exists only while the user opens the Legacy Viewer migration. Its independent source catalogue never changes the startup fallback decision or native selection; missing originals join the existing bounded item-fetch queue. The modal selects one exact source set and an unused native name; cancellation is read only, disappearance never selects another source implicitly, and the request rechecks inventory/services before planning ordinary native folder/link creation. The existing pending AIS acknowledgement remaps the new selected UUID. No source write/upload occurs, the four actual legacy swim mappings are copied when present, and the existing native default remains intact. The shared completion runs for both manual migration and initial reconstruction before ordinary native folder/link creation. It uses the maintainer-approved donor mappings, leaves unavailable donors explicit and never changes the swimming preference or an existing native set.

Approved AO editor integration (2026-10-16): `ui/ao/editor.rs` renders a scrollable state rail and a focused detail pane, with one swimming slot for the selected state. Sit options follow activation; main entries use single-click choice and drag ordering. `ui/ao/combinations.rs` provides a native guided dialog, explicit group/track contexts, add/rename/back/dissociate controls, and independent track cycles. All surfaces, typography and controls retain the shared theme and palette. `ao/inventory/editor.rs` plans bounded native folder/link mutations: destination links precede removal of old Aurora links, originals remain read only, dissolved folders go to Trash, and no ambiguous failure is replayed. Main steps, group/track members and tracks retain their order on reload; tracks use an optional `OR` folder parameter alongside `TR` and their cycle suffix, preserved by renaming and cycle saves. Pending editor selection is bound to its original set/state and remapped only after acknowledged AIS creation; an intervening navigation is respected. Existing AO runtime and AgentAnimation own simultaneous groups and independent tracks without a second animation owner. Independent movement controls use side-by-side run/flight factors, obstacle passage before swimming, and EXPERIMENTAL labels; accepted physics and bridge protocol are unchanged. The extra standing entries and combination examples exist only in explicit offline `interface*` fixtures, never in user inventories. The water state rail uses the shared amber warning triangle for missing required slots, a green check for assigned required slots and no marker for optional slots. The detail pane and swim switches share the same concise missing-animation warning; the optional-slot count is removed. `interface-swim-missing` provides an offline incomplete-set preview.

Aurora folder naming (2026-10-17): native roots are `#Aurora/Aurora AO` and `#Aurora/Aurora Bridge`. Discovery prefers these names and falls back to previous native `AO` / `Bridge` folders only when the new sibling is absent. Startup renames the old folders in place with acknowledged AIS patches, preserving all descendant IDs, selected AO set, swimming assignments and the bridge folder UUID embedded in its handshake. An old AO, including an empty one, remains authoritative and cannot trigger legacy reconstruction. An existing new sibling wins without merging/deleting the old one. Failed/ambiguous renames are not replayed automatically in the session; ordinary Reload/Repair uses refreshed inventory. Both generations of interrupted reconstruction staging remain guarded. The LSL, movement motors and legacy inventory protection are unchanged.

Cached bridge provisioning readiness (2026-10-17): `World::inventory_writable` shares the AO startup gate (completed arrival, no crossing/cache hold, nonempty metadata-write and descendant-readback services) with the bridge factory. Provisioning waits before sending edits or spending the stage timeout; pending edits retain their deadline and errors are never blindly replayed. This covers in-place old-folder renames as well as fresh installation. Active bridge physics and authenticated read queries remain independent of inventory-service availability. The offline transport now models unavailable services instead of accepting an impossible startup edit.

The import layout uses `imports/mesh/inspector.rs` for bounded actual worker
diagnostics and cached per-model joint overrides. `imports/ui/mesh_panel/panels.rs`
contains the Avatar dependencies/read-only position tables and Journal view;
`mesh_panel.rs` keeps the metadata/preview header and model/fee footer fixed
around the scrollable five-tab body. Geometry processing and fee protocol
remain on their existing worker/session paths.


## Local interactive water

AO quick controls (2026-10-18): `ui/ao/quick.rs` opens above the toolbar AO
button (or beside a relocated side-bar button), using the shared overlay and
popup behavior. Its set selector and enable switch edit the same account as
the manager; swim/water-walk and run/flight boosts edit existing movement
preferences and retain the selected set's swimming-readiness gate. Child set
menus keep the parent open; Escape/outside clicks dismiss it, and the manager
button opens the existing floater. `ui/ao/category.rs` partitions every state
into General, Ground, Jumping, Flying and Swimming, in that order. General
contains Standing, Sitting, Sitting on ground, Standing up, Typing and Always,
with the sitting states immediately after Standing. Ground groups walking/running,
crouching and turns. Jumping orders preparation, jump, fall, soft landing and
hard landing, and initially selects Jumping. Flying orders hover, slow flight,
flight, descent and ascent. Underwater floating immediately precedes
underwater swimming. The editor's
category tabs remember their selected state, follow external state links, and restrict rail
keyboard navigation to that category. The rail reserves vertical arrows and
retains pending focus while scrolling to an offscreen state. Inventory folders
and runtime states are unchanged.

AO water states (2026-10-17): the canonical `Swimming Forward` inventory folder
is the single surface swimming list; `Underwater Swimming` is an independent
list with an explicit surface-list fallback for old sets. Water states share
the ordinary AO editor, groups, tracks and cycles. `ao/bootstrap.rs` plans one
acknowledged consolidation per selected set/session from old crawl/backstroke
folders, moving AO links/containers and preserving source folders and foreign contents.
Original animation items remain untouched. `ao/swimming_tests.rs` covers depth,
loading handoff, list selection and local/observer transition consistency.
The AO prefetches four unique assets per frame after an assignment revision.
`AgentAnimationChanges` carries stops/starts in the same bounded reliable
message; the runtime retains the underlying locomotion context when its own
default-animation stop is echoed. Simulator jump/landing control clips remain
active. Missing looping-clip echoes can be repaired every two seconds;
one-shots are never replayed by that repair path.

Local surface flotation (2026-10-17): `movement/water_float.rs` filters the
rendered macro-wave displacement with a short exponential response and a
bounded vertical speed. `app/movement.rs` reads the movement controller's pure
swim intent (or the resolved AO state without a motor): idle surface float,
surface swimming in any horizontal direction enables it, without flat-target convergence,
terrain availability or velocity thresholds. A broad body/water intersection
guard and explicit submersion, seats, dives, entries and departure exclude it.
Inspecting an established surface hold preserves its visual heave while the
motor is suspended. Intent queries do not advance AO animation clocks.
`World::water_visual_dz` supplies this self-avatar-only offset to the common
draw transform and camera frame. Body geometry, posed anchors, attachments,
picking and contact positions therefore agree. Simulator positions, raw
unshifted transforms, saved hover/float settings and bridge targets retain
their existing values. Teleports, region changes, long gaps and seating clear
stale state; leaving the surface or disabling waves settles to zero. Carried
heave is removed from anatomical material velocities so waves alone do not
create splash/bubble streams. Only the macro spectrum drives the body: its
own fine interaction ripples cannot feed back into flotation.


Anatomical contact revision (2026-10-17): `scene/water_contacts.rs` replaces the
single avatar-waterline emitter with seventeen independent fitted collision
ellipsoids: feet, lower/upper legs, hands, lower/upper arms, pelvis, belly,
chest, neck and head. Current shape-adjusted palette matrices place each one.
A water-plane cross-section supplies the local contact, size and displaced
area; shallow submerged motion attenuates with depth. Material velocity uses
the same local point under successive transforms, so a waterline sliding along
a stationary tilted limb cannot create a false wake. Fast vertical crossings
are swept between frames. Each moving part leaves a short fine-atlas trail;
a downstroke/entry or head emergence produces its own small expanding ring and
spray. There is no root-level bow emitter. Only the moving submerged head owns
sparse air, with the existing 1.25 s budget.

Shoreline footfall refinement (2026-10-17): continuous wake pressure uses the
interval's squared speed with bounded high-speed response and linear displaced
area, preventing small overlapping parts and brief animation peaks from making
large furrows at walking speed. Feet use separate lift/contact hysteresis instead
of repeated submerged downstrokes, including an upward heel lift that only
partly emerges. Each descending contact emits one shallow
ring and a low pair of broken water sheets with sparse ballistic rim droplets;
strength follows landing speed, foot area and available water depth. Dragging,
idle wave crossings and dry ground produce no foot slap. The footfall flag
reuses events 7/8, existing draw instances, quality budgets and independent
spray/displacement switches. This is a visual extension of the independent
left/right contact transitions in `LLVOAvatar::updateFootstepSounds`
(`indra/newview/llvoavatar.cpp`), with no sound or movement change.

The CPU contact surface inverts the same Gerstner horizontal displacement as
the rendered spectrum, using the GPU's bilinear bathymetry and shore damping;
a production-WGSL GPU comparison guards all four qualities and shallow/deep
water. Per-avatar sampling is shared across the frame workers, with fine parts
limited to the local water patch. Visibility is rebuilt on the bounded candidate
cadence: native head/upper/lower draw records and the nearest anatomical ancestor
of visible rigged-mesh skin influences select the active parts. Fully hidden
bakes/meshes and HUDs supply no coverage. These are fitted animated volumes,
not per-triangle fluid collisions: unusual mesh silhouettes and partial texture
alpha holes remain approximate. No full mesh skinning, GPU readback, network
message or physical force is added. Independent strike/spray clocks preserve
water displacement when particle switches change. The shared event caps are
64/96/160/192, with two-second part wakes and entry-sheet priority.

Avatar splash refinement (2026-10-17): dives and cannonballs share a radial
entry sheet, with six smaller dive sectors or ten larger cannonball sectors.
Each packet uses two curved, subdivided patches and leaves instances for ballistic rim
droplets even at Low quality. The sheet uses its interpolated normal for
dielectric reflection/refraction and broken translucent coverage; its foamy rim
expands as the sheet falls. Limb downstrokes and body translation emit low,
directed lateral fans instead of radial fountains. Each anatomical contact follows its own posed material motion;
the furrow follows swept world-space contact intervals instead of stamping a
foam V along the latest heading. `water_wake.rs` / `water_wake.wgsl` rasterize
those intervals additively into the lower half of the existing water atlas:
a 32 m local patch at 128–512 resolution (25–6.25 cm cells). It stores slope,
height and restrained breaking foam. The central depression refills while
shoulders and secondary ripples spread outward, staying on the old path after
a turn or stop. Length-normalized kernels keep the reconstructed ribbon stable
across emission/frame rates. One small pass touches only segment bounds;
coarse compute excludes these historical segments. The atlas keeps the existing
scene bindings/samplers, with no surface-pixel event loop or GPU readback.
Fine height displaces the denser 64–224 grids in close 64 m patches (within
24 m of the camera); ordinary distant grids remain 32–96. Fine normals,
refraction, Fresnel and local micro-wave damping reveal the furrow even with
spray/bubbles or contact foam disabled. Foam remains independently switchable.
The same terrain/water-level guards reject dry or mismatched region water.
Spent displacement events expire after integration; droplets, entry sheets and
bubbles retain only their respective flight/lifetime windows. At the quality
event cap, an active dive/cannonball sheet and foam ring take priority over
ordinary contacts, so Low quality retains a complete entry silhouette.

`aurora-render/src/water_effects.rs` and `shaders/water_sim.wgsl` maintain a bounded, world-anchored GPU height/velocity/foam field (128–512 square, 128–256 m extent). A damped wave equation propagates contact impulses; foam advects and decays. The field scrolls by integer cells, clears on region changes/large jumps, and never reads back to the CPU. Bathymetry is sampled in parallel into a shared 64-square snapshot. The master switch and per-effect choices are in Preferences › Aurora › Water (`WaterEffectsSettings`), independent of GTAO and the existing planar-reflection controls.

`aurora-render/src/water_optics.rs` schedules a reduced-resolution exterior
colour/depth view only when the submerged camera's field can contain a Snell
window. Its wider auxiliary frustum contains opaque/masked air geometry missing
from the main image. Its apparent eye depth follows the central refraction
angle; two depth reprojections refine parallax in `env.wgsl`.
Low/Medium/High/Ultra use quarter/three-eighths/half/three-quarters
linear resolution. The capture reuses reflection pipelines and the existing
water draw bin, whose GPU candidates are the union of reflection/exterior
frusta; each pass raster-clips to its own view. It renders neither water nor
new shadow/GTAO passes and adds no readback or main-scene copy. The source has
no water fog, so only the true eye-to-interface segment attenuates it. GPU
Water timing includes this capture and the interface depth prepass. Physical
Fresnel/total internal reflection remain intact. This single exterior depth
view improves offscreen coverage but does not resolve transparent layers or
all disocclusions around nearby geometry.

`scene/water_effects.rs` selects nearby object footprints and displayed avatar joint contacts with bounded counts. It emits local events from actual displayed motion, rejects teleports, and retains events only through their effect lifetime (at most three seconds). The renderer evaluates droplet/bubble trajectories on GPU; submerged particles enter the refraction source before water, airborne particles follow the water. No simulator commands or forces are generated. Terrain, static and moving object foam, crest foam, ripples/wakes, caustics, geometric waves and physical Fresnel/absorption have separate switches. Coarse object footprints approximate arbitrary meshes; custom mesh/prim water is not inferred as a water volume.

The scene's water tiles share quality-dependent grids near the camera and flat quads at distance; all use the same world-space wave phase. Caustics light receiving opaque surfaces before water fog/refraction. The opaque prepass and its GTAO input remain unchanged. `demo_water.rs` is the offline `AURORA_DEMO_WATER` scenario, using ordinary synthetic object updates.

Water realism revision: `water_spectrum.wgsl` shares the metre-scale Gerstner derivatives and periodic capillary spectrum between the visible surface and `water_caustics.wgsl`. Refracted triangles accumulate conserved source-area light into one 256-square HDR map, with the analytic optical Jacobian smoothing triangle density and a finite focus width limiting peaks. RGB encodes three receiving depths; opaque receivers sample it with depth interpolation, sun projection, extinction and shadows. Nearby water tiles subdivide into 64 m patches, preserving shared boundary phase while distant parents stay quads. Sub-grid limb/contact waves affect normals; the finer avatar atlas also displaces close geometry. Zero-volume contact impulses, divergent hull wakes and stern foam feed the bounded simulation. Shore fronts follow bathymetry; actual opaque-depth intersections refine proxy foam, with filtered continuous coverage and wet terrain at the shoreline. Foam uses a world-stable integer hash, four-tap cubic reconstruction and a known-obstacle mask to avoid regular pore lattices, grid contours and false terrain contacts. Displacement events are distinct from short impact/downstroke spray; only a moving submerged head releases a sparse air packet. Particle emitters are packed before particle-free displacement events, with newest-first budget priority. All effects keep their existing independent controls.

Surface-motion/underside revision: an avatar body-waterline contact is derived from its displayed feet/head envelope, so horizontal run/swim motion still drives local part trails and modest spray with flat custom poses. Dry bodies and bodies well below the surface have no such emitter. Horizontal poses include body thickness and near-surface wrists; the sampled submersion attenuates their wake, and submerged bodies cannot throw bow spray from depth. Head air remains bounded and idle-suppressed (2–4 bubbles per packet, minimum 1.25 s interval). Water fog clips rising underwater rays at the water/air exit. The underside retains quality-bounded reflected submerged rays against the existing opaque depth/color (12–30 steps, fixed 200 m range, four hit-refinement steps); source fog is removed before applying the actual surface path once. Transmission uses the reduced exterior capture described above, with screen-space reconstruction as a fallback when that capture is inactive. A blurred bathymetric bottom-radiance projection stabilizes terrain reflection, with water scattering when no submerged source is visible. Reflected sharp geometry remains screen-space limited; the exterior capture is also a single-depth approximation. The Fresnel switch controls Snell/Fresnel behavior; disabling it retains transparent baseline transmission. Refraction and dielectric Fresnel follow [PBRT, Specular Reflection and Transmission](https://pbr-book.org/4ed/Reflection_Models/Specular_Reflection_and_Transmission).


Water interface revision (2026-10-18): the actual displaced mesh uses hardware
near-depth clamping, with a perspective-preserving fragment-depth fallback
near water on unsupported adapters. A depth-only GPU prepass chooses the
closest wave interface by monotonic view distance, including inside the
normal near plane; water shading samples this separate single-sample depth
while preserving the opaque scene depth for submerged alpha geometry. The
prepass reuses the indirect water draws, runs only with displacement, and is
included in GPU Water timing. Opaque optical segments retain immersed
endpoints from either side of the camera-local boundary; interface hits use
the actual known water segment and undo source fog before refraction.
Coloured attenuation follows water distance without a camera-depth fade,
and submerged lighting stays consistent across camera crossings. Sky/EEP
parameters and physical swimming remain unchanged.

Water action presentation (2026-10-17): `movement/water_jump.rs` predicts a
bounded native jump from the current velocity and terrain, with the existing
scene picking hierarchy rejecting known solid walls, decks and roofs. A jump
alone retains its ordinary pose. A fresh descent edge within 450 ms of the jump
signal arms configured dive clips toward a clear deep-water landing; a second
edge switches to the optional cannonball. A later descent edge can also arm the cannonball directly
while airborne above water, including flight, without a previous jump or predicted
landing. This releases native flight; resuming flight after release, teleports,
bridge loss, seating, lock and suspension cancel it. No fall force or avatar
relocation is introduced. Bridge protocol 1.25
adds passive modes 11/12 carrying only height and a bounded action nonce; native
controls pass through, repeated requests cannot renew an action's deadline,
and existing acknowledgements report states 12/13/14. Dive observation lasts at
most five seconds; cannonball observation allows a bounded 120-second long fall.
Arming permits unknown terrain and shallow water, rejecting known dry terrain;
the entry burst still requires actual body contact over sufficiently deep water.
The AO remains the sole
AgentAnimation owner. Actual posed head/feet/hands/pelvis determine first water
contact and one bounded local spray/ring event. Recovery temporarily suppresses
priority-six AO replacements so the supplied priority-five entry can finish.
The keyframe controller follows LLKeyframeMotion::setStopTime: loop tails play
before ease-out, including a zero-length held loop with a full authored outro.

The eight original water states still gate the swim motor. Independent
underwater forward swimming, dive jump, dive entry and cannonball bring the
visible catalogue to twelve. Horizontal input uses one surface state or the
underwater state according to depth, with ordinary AO list/cycle selection.
Old style assignments are consolidated into the surface list. Jump presentation requires its configured, decoded optional clips;
no bundled asset is used as an online fallback. `ao/water_demo.rs` is an explicit
offline-only read-only native pack decoder, ephemeral catalogue and trajectory
fixture. `scripts/author-water-cannonball.py` authors the body-only native clip
for that fixture and human-controlled import later; it has no upload API.

Cannonball recovery uses the configured native loop phase at actual stop time,
including the remaining intro if PageDown arrived just before impact. Repeated
PageDown presses during one airborne action neither renew its deadline nor restart
the pose. The impact reuses the particle pass: each of six dive or ten cannonball
radial packets spends two instances on curved, torn crown patches, leaving the
rest as droplets. Foam/ripple injection stays in the existing bounded compute
field. The underside floor fallback approximates a rough multiple-bounce lobe
continuously across grazing ray directions, avoiding alternating blue holes.

Water walking (updated 2026-10-18): `movement/water_walk.rs` supplies a separate,
per-account opt-in physical support controller. Bridge mode 13 steers toward
native water height plus half the avatar height, with collision-aware targets
bounded to 1.6 m; it uses ordinary 3.2/5.13 m/s walk/run speeds. Descend selects
Crouching at rest or CrouchWalk while moving, with the existing ground crouch
speed of 1.2 m/s taking priority over run. Bridge 1.27 keeps water support during
Descend and caps speed directly from captured controls before the next viewer
command. It releases for dry terrain, flight, seats, suspension or bridge loss.
Movelock retains its normal anchor ownership. A brief ascent uses the existing
bounded jump pulse. The bottom-toolbar `water_walk` button and Movement
switch share this preference. Existing nonempty custom bars receive the button
once; deliberate empty bars, side placement and later removal are preserved.
The ordinary AO standing/walking/running/crouching selections, groups and cycles own
local and AgentAnimation playback. Disabled/paused/missing AO slots use standard
motions through that same owner, with stock ownership removed immediately on
release. The visual wave follower accepts these supported ground states too,
including while Descend is held. Acquiring support counts as grounded for water
gestures immediately, preventing a crouch from arming a cannonball.

Bridge 1.24 deepens the surface descent goal to `-max(height/2 + 1.1, 1.8)` m
relative to water. Modes 7/8 share a bounded 4–6 m/s vertical speed limit,
with proportional braking near the goal and a combined target budget independent
of horizontal swim speed. Ascent ramps at 8 m/s²; explicit descent responds
immediately, like the former dive motor. The existing control gestures,
collision steering and command watchdog remain authoritative.

Water response follow-up (bridge 1.25): a captured surface-mode Down edge
selects the bounded dive goal directly in `surfaceDive`, before waiting for
HTTP. It clears the old surface target and executes on the same control event;
descent uses a 0.12 s target horizon/response with the existing speed and distance
bounds. A one-second guard prevents an older in-flight surface refresh from
undoing that edge; explicit return/release commands take precedence. Controls
do not renew the HTTP watchdog. The viewer preserves recent acknowledged capture
across compatible motor transitions, including superseded HTTP refreshes, while
still sending immediate release even when it matches an older idle acknowledgement.

A water-origin ascent hold suppresses native AutomaticFly until release, even
after reaching land. A reachable platform takes precedence over flight at the
surface hold threshold; open-water deliberate flight remains available. Water
jumps retain an explicit nonflying end state, and an old pre-swim flight flag is
never restored onto measured support. Native jump signals refresh the early
dive-gesture window so server delay does not consume it before the jump begins.

`world/received_items.rs` files only newly observed inventory offers when the experimental preference is enabled. It waits for legacy owner names and acknowledged ordinary folders below `#Aurora/Received items`, moves direct offers by their delivered item/category ID, and supplies the exact destination in direct and scripted acceptance buckets. Refusals cancel pending moves; disabling or failed folder/name resolution preserves acceptance through its original system destination. Fully fetched empty sender folders move to Trash while filing is enabled; pending deliveries protect their destination. The Received inventory view joins FT_INBOX (presented as the first MarketPlace folder) and the sender-folder subtree with view-only parents, preserving server Marketplace membership. `ui/inventory/received.rs` caches unread delivery IDs by inventory generation for the compact tabs and wide sidebar; sender containers and nested gift contents are not deliveries. Its known/unread UUID sets persist in `accounts/<agent>/received-<inventory-root>.json`, outside the disposable cache; `received/storage.rs` performs ordered atomic writes and reads off the frame thread. The first complete snapshot establishes old history without a red badge, fetching only the delivery containers, including while the inventory window is closed. Exact direct receipts and scripted destination snapshots preserve arrivals during that baseline. Displayed search results acknowledge individual items/categories; reads survive logout, and a new UUID arriving online or offline counts even when its asset creation date is old.
