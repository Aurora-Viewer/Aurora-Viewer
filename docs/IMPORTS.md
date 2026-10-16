# Asset imports — Legacy Viewer reference

This matrix tracks the imports from disk and their local previews.
It is used to check the options, the default values, the protocol and
the errors, before the human validation on the grid. A window that is
displayed is not enough to consider its import finished.

Scope confirmed on 2026-10-12: textures, materials, meshes, sounds,
animations, bulk uploads and local previews. The imports of linksets, EEP
and Windlight are listed for later tasks in `TASKS.md`.

Reference consulted: the Legacy Viewer `d942e1118d01c3383ef7d6b64f248af77452b0dc`
(2026-10-12). The visible choices follow this revision of the sources.

## Common workflow

- Choose one or more files; name and description; destination folder per
  type, including the five existing custom folders.
  The destination picker offers a filter by name, without changing the
  selected folder while typing. The destinations respect the inventory
  protections; their list is cached and virtualised.
- Compact list: one line per file, without a "Ready" mention; progress,
  success, cancellation and error shown by an icon with a tooltip.
- Upload permissions: next owner (modify, copy, transfer), group and
  everyone, independently per type in Settings; do not grant rights that
  the server has not confirmed. Settings contains only this table and payment.
  Saved common sharing flags migrate as per-type fallbacks until a row is edited.
- Type icons open the disk picker when their list is empty, otherwise their
  existing draft. Clicking the displayed type closes its inspector while
  leaving the launcher visible.
- Price taken from the account benefits and the region capabilities;
  sufficient balance; explicit confirmation before each paid operation.
- Analysis, conversion and encoding in the background; progress,
  cancellation, errors per file; no automatic retry of an upload whose
  result is unknown (risk of being charged twice).
- Server response then inventory re-read before declaring success.
- Invalid files, dimensions/durations/sizes out of limits, missing
  capabilities, refusal and timeout: keep the draft and explain the error.

Sources: `llviewermenufile.cpp`, `llviewerassetupload.cpp`,
`llfloaterperms.cpp`, `llagentbenefits.cpp`, `panel_preferences_uploads.xml`.

## Textures

- Formats of the Legacy Viewer file dialog and RGB/RGBA validation.
- Dimensions before/after resizing, maximum X/Y dimensions, powers of two,
  2K and the associated fee; bulk optionally limited to 1024.
- Lossless compression for small textures (area ≤ 128²), `LosslessJ2CUpload`
  setting; upload JPEG2000 for the others.
- Detection of an almost opaque alpha channel, removal offered and
  deliberate keeping after a warning.
- Image preview with pan/zoom; hair, male or female head/body, skirt and
  sculpted prim with rotation/zoom.
- Former temporary texture checkbox: visible in the Legacy Viewer only without
  server-side baking, but `TemporaryUpload` is no longer read by its upload
  path in this revision. Current SL regions use baking; Aurora offers no
  fake free upload. Local previews stay free and send no asset to the
  server.

Sources: `llfloaterimagepreview.cpp`, `floater_image_preview.xml`,
`llviewertexturelist.cpp`, `llimageraw.cpp`.

## Sounds

- WAV: PCM restrictions, channels, sample rate, bit depth and duration;
  conversion to the Vorbis sound of Second Life, with the Legacy Viewer's
  parameters.
- Name/description, fee and destination; local sound preview.
  The Play and Stop controls are placed side by side.
- Aurora extension requested by the human: edit the selected interval and
  import gain (0–400%) without changing the source. Amplified PCM is clipped
  to the digital ceiling, with orange waveform peaks and an explicit indicator.
  Audition uses the same processed PCM as upload; committing a gain adjustment
  restarts the current audition with that result.
- Optional maximum piece length creates consecutive `_01`, `_02`, … inventory
  sound assets from the selected interval. Boundaries use sample indices, with
  no gaps or duplicated samples; the last piece can be shorter. The confirmation
  includes every piece's fee. Each verified receipt is kept, so restoring a
  failed split draft does not upload its acknowledged pieces again.
- Long sources up to 600 seconds and 16 MiB can be decoded for editing;
  sources above the grid limit automatically enable splitting. Each resulting
  asset retains the grid's 30-second (SL) or 60-second limit. WAV format,
  mono conversion and Vorbis codec rules are unchanged.

Sources: `llfloaternamedesc.cpp`, `llaudio/llvorbisencode.cpp`,
`floater_sound_preview.xml`.

## Animations

- `.anim`: validation without altering the data, in particular the
  constraints. Negative loop markers and other finite time values from
  exporters are accepted as by `LLKeyframeMotion::deserialize`.
- BVH: hierarchy, channels and axis order, reference pose, translation of
  the joint names, removal of fixed tracks, key reduction.
- Priority 2 by default; loop disabled; in/out as a percentage or in
  frames; fades of 0.3 s; hand poses and expressions.
- Local preview: standing/walking/sitting/flying, play/pause/stop, time
  slider, camera, reload of the file, statistics and parsing errors.
  Clicking an animation in the list restarts its playback from the
  beginning, including if it had already been played or stopped.
  "Also play on my avatar" is shared by all the animations and saved in the
  viewer settings, enabled or disabled. The checkbox is placed below
  the playback controls.
  Pause and the slider also drive the preview on our avatar; the ends of
  these local motions produce no network stop.
- Bulk imports: `.anim` accepted, BVH excluded like the Legacy Viewer.

Sources: `llfloaterbvhpreview.cpp`,
`llcharacter/llbvhloader.cpp`, `llcharacter/llkeyframemotion.cpp`,
`floater_animation_bvh_preview.xml`, `floater_animation_anim_preview.xml`.

## PBR materials

- glTF/GLB: choice of the material or import of all the materials of the
  document.
- Base colour/RGBA, normal, metallic/roughness (G/B channels), emissive,
  factors, opaque/mask/blend, alpha cutoff and double sided. The texture
  transforms are reset to zero for the inventory asset as in
  `sanitizeAssetMaterial`; they belong to the overrides of the faces.
- Removal of the alpha of the technical maps and separate occlusion resized
  then merged into R in ORM; R is 255 without occlusion. An occlusion map
  alone creates an ORM map that is white in G/B.
- External/embedded/local textures or inventory UUIDs; maximum resolution,
  removal/replacement, detailed cost of the textures.
- Reuse the shared textures; send the textures before the material, then
  replace their references with the confirmed UUIDs.
- Material with no fee of its own; creation and saving as a `material`
  asset, LLSD envelope version 1.1 / `GLTF 2.0`.
- Local preview and saving of the imported material with the permissions.
  The properties are placed next to the preview; the four maps use aligned
  columns in a window fitted to the content; long names are truncated, with
  their full text on hover.

Sources: `llmaterialeditor.cpp`, `panel_gltf_material.xml`,
`llprimitive/llgltfmaterial.cpp`, `lllocalgltfmaterials.cpp`.

## Meshes

- Collada and glTF/GLB according to the filters of this Legacy Viewer version;
  instances, materials, coordinates, units, transforms and rigging.
- Four LODs: file, generation, reuse; counters and validation of the
  faces/instances, suffixes and search for the associated files.
  The window shows the preview to the right of the name, description and
  destination fields; three sections, LOD, Collisions and Upload, sit below.
  The High LOD is shown by default, without a second level selector.
  Help texts, counters and statistics use the normal text size.
  In Upload, the options of the model and those of the upload take two
  columns; the five suffixes are aligned below in a grid fitted to the
  available width. The physics preset stays on a full line. The camera
  controls are shown when hovering the model.
  Each LOD has a framed block with title, source, file and number of
  triangles; clicking its level shows it in the preview. Changing a source,
  a mode, a threshold or a budget also shows the LOD concerned. Hovering
  the counter shows its proportion of triangles relative to the original.
  Dragging the numeric fields stays continuous at the edge of the screen,
  with the cursor kept near the field; a click still allows typing.
  The visualisation tools and the advanced settings stay accessible without
  hiding the model. The fee calculation is in the window footer.
  The checkboxes are square and contrasted; the menu rows keep their size
  on hover and when the selection changes.
- LOD generation: methods, triangles, error and smoothed normals; values
  and presets of the reference window.
  The triangle mode uses an internal threshold of 100% like the Legacy Viewer; the
  saved error threshold only applies to the error mode. AUTO welds the
  identical exported vertices in both modes, like the Legacy Viewer, then removes
  the unused copies before simplification so that they are not still
  treated as locked seams.
  The threshold is shown as a percentage of the model size; its help
  explains the trade-off between fidelity and simplification. It limits
  the deformation, without imposing a number of triangles; preserving the
  normal and UV seams can leave almost all the triangles, even with a high
  threshold. The fast method accepts more simplification at the seams.
  AUTO also progressively reduces the normal and UV seams, then uses the
  fast method if necessary, like the Legacy Viewer when the budget is exceeded by
  more than 80%. The faces keep their material slot.
  The "File" choice opens the picker before changing a source that has no
  file; cancelling keeps the old LOD. Loading a valid file or changing
  level unblocks a preview that had failed, without accepting an old result.
  During the recalculation, the old model stays displayed and the slot keeps
  its height; changes close together are grouped before the worker is
  started. The type and section tabs use those of the inventory.
  A mesh refusal keeps the HTTP status, the code and the validation messages
  provided by the server, in a bounded area. Only these fields are read;
  URLs, paths and control characters are removed.
  The four MAV validation codes explained by the Legacy Viewer are also
  explained in French and in English.
  The mesh import rights are reported separately. A failure after the
  transfer keeps the unknown-outcome warning, its available details, and
  triggers no new upload.
- glTF palettes: recognised joints, weighted joints first then unused
  joints of the active groups, limit of 110 like the reference.
- Physics: file/LOD/preset, convex analysis, simplification, parameters of
  the methods, progress and cancellation; limits of the hulls, vertices,
  degenerate triangles and thin objects.
- Scale/dimensions, inclusion of the textures, joint weights, joint
  positions, scale lock and pelvis offset.
- Preview: LOD, edges, textures, UV guide, exploded physics, weights,
  joints and overrides; camera and colours saved.
- Degenerate physics triangles: `ll_is_degenerate` tolerance, diagnosis
  and highlight colours; quote refused as long as the raw physics stays
  invalid. The instance parameters keep the LODs and hulls cached.
- Prior server calculation of the weights and of the price, breakdown of
  the price/streaming/physics, authorisation state of the account, reset
  and log.
- Upload of the asset and of its instances through the Legacy Viewer mesh
  protocol.

Sources: `llfloatermodelpreview.cpp`, `llmodelpreview.cpp`,
`llmeshrepository.cpp`, `llprimitive/llmodel.cpp`,
`floater_model_preview.xml`, `menu_model_import_gear_default.xml`.

## Bulk uploads and other imports of the reference

- Mixed bulk upload: textures/sounds/`.anim`/materials, announced
  exclusions, total before confirmation, 1K limit, progress and individual
  errors.
- Local loading of textures, materials and meshes in a single picker,
  without changing mode; glTF/GLB documents are recognised by their content.
  Shared commands: apply to the selection, remove from the selection and
  place a support prim. Textures and materials apply to the chosen faces
  (or to all the faces), without changing the server data; the removal
  restores their current state. Refresh after a change to the file and no
  charge. The Close button of the window footer is removed.
  The preparation uses the staging pool kept by the scene and the GPU
  publication goes explicitly through the render loop, including when the
  interface does not hold the graphics context. The removed slots are
  freed by this same loop.
- The local library stays saved between launches in
  `local_previews.json`, inside the private profile of the viewer. It keeps
  the references to the sources, their identifiers, the texture/PBR/LOD
  settings, the custom maps, the watching and the preview camera; no pixel
  and no source mesh is copied. The applications to objects are separated
  by account and grid and are restored when the objects arrive.
  A missing file stays in the list with an error and keeps its settings.
  "Remove" deletes its reference and its saved applications, without
  erasing the source file. The changes are grouped on a dedicated writer,
  replace the file atomically and are flushed at logout or normal closing.
  An unreadable library is preserved.
  This persistence is an Aurora improvement requested by the user.
- The build picker exposes the published local diffuse textures and PBR
  materials. Choosing their row applies the overlay without a network
  command; OK keeps the link, Cancel restores the previous local
  appearances, including on a mixed selection. Switching to an inventory
  asset removes the overlay from the faces concerned only. These links
  follow the same persistence as the commands of the import window. The
  pickers of technical maps, sculpt and projector stay reserved for
  inventory assets.
- Local mesh: choice of the model, optional units in metres, imported
  dimensions, application to/removal from the selection and support prim;
  watching disabled by default, period of 3 s adjustable from 0.1 s like
  the Legacy Viewer.
  The preparation and the installation of the mesh data are independent of
  the graphics context held temporarily by the render loop: applying,
  reloading or placing a support prim also works from the interface.
- Environments: EEP settings and old Windlight presets, multiple imports.
- Legacy Viewer linkset backups: rebuilding of the prims and of the contents,
  reuse/upload of the assets, temporary texture options, offset.
  This workflow also relies on the build tools and their acknowledgements.
- The existing CSV imports of parcel access and AO imports remain separate
  workflows and must keep their behaviour.

Sources: `llfloaterbulkupload.cpp`, `llviewermenufile.cpp`,
`lllocalbitmaps.cpp`, `fslocalmesh*.cpp`, `llsettings*.cpp`,
`fsfloaterimport.cpp`, `floater_fs_import.xml`.

## Validation

For each line: pure rules and codecs tested offline; local HTTP server for
negotiation/upload/confirmation/refusal; screenshots of the windows and demo
workflows; then tests by the human of the prices, rights and results on the
grid. No real connection or operation on the grid by the agent.

Status: workflows and test viewer validated by the human on 2026-10-12,
with agreement to merge. The codecs,
the HTTP/UDP network paths, the drafts and the GPU previews are present.
The texture, sound, animation, material, mesh and local application
screenshots have been examined: preview to the right of the fields for the
mesh and of the parameters for the material, above for the other types;
actions and cost accessible in the window footer. The tests cover in
particular the ORM preparation, the three V-HACD modes, the LOD weights, the
glTF palettes, the scale limits, the reloading of the dependencies and the
keeping of the acknowledgements after cancellation. The overall validation
of the viewer is not a detailed record of the paid uploads on the grid:
prices, permissions, created inventory and rendering of the assets remain
in the test protocol below. No result per type is claimed without a human
report.

Local checks: `check.ps1` green, 1,498 tests passed, 22 ignored, Clippy
without warnings. Profiling of the demo scene: average interface CPU over
the last three periods, 0.37 ms without the window then 0.57 ms with the
PBR preview (0.51 ms animation, 0.51 ms local mesh). The processing of the
files and of the physics stays on the workers.
After clarification of the mesh window: LOD / collisions / upload options
screenshots, local application and animation label examined; average
interface CPU of 0.57 ms over the last three periods of the mesh scenario.
The network settings and the preparation of the assets remain identical.

After unification of the local previews: mixed selection of the three
types, GPU application, removal and detection of the support prim checked
in the demo for textures and materials; application of the local mesh
checked too. The tests cover the buttons of the three types, the glTF
detection and the restoration of the server state after an object update.

The human fixtures were also read and prepared offline: nine meshes with
their four LODs and their physics, 58 textures, a 1024 × 1536 image and a
WAV of 0.3 s. The associated files are loaded with the custom suffixes
`lowest/low/medium/high/physics`. The triangles of each LOD are kept and
the mesh requests are encoded without being sent.
Another batch of 79 native animations was validated: original bytes kept
and poses computed without any non-finite value. The aggregate counts
remain separate from `check.ps1`; the personal files and their inventory
are not incorporated into the repository fixtures nor into the published
traces.

The branch was synchronised with the 26 PRs merged since PR 154, up to and
including PR 181. The imports keep the hierarchical inventory picker, the
selection of invisible faces, the new GPU anchors and the time evaluation
of their animation previews. The persistence tests check restart, settings
of the three types, custom maps, isolation of the applications,
missing/found source, explicit deletion and corrupted save file kept.

### Human tests

Clicking **Build → Import** toggles the compact import chooser, exactly like
the configurable Import shortcut (Ctrl+U by default). It is not in Contents.
The chooser has individually frosted icon buttons without a common bar
background. Clicking a type opens its dedicated window and filtered file
picker: a file list at the left, metadata, preview and relevant controls at
the right. The icon bar stays visible and clickable after choosing a type;
Ctrl+U, Build → Import or its close cross can hide it independently of the
inspector. Dedicated windows have their own saved sizes and positions,
without unrelated type tabs or permissions controls. All permissions are
edited in Settings, with per-type next-owner rights and common Share with
group / Everyone can copy options. First-opened windows use the same corner
at the left; windows previously shown retain their saved positions.
**Multiple** (formerly Bulk) retains the complete window and all its tabs;
changing a tab inside it never reopens the picker or changes window identity.

Go through the type buttons and the tabs inside Multiple:

- **Textures / materials**: choose a file, check dimensions and alpha,
  edit the PBR maps and factors, rotate and zoom the preview. Inventory
  materials use glTF/GLB; the JSON recipes of an assembly package are not
  importable glTF assets.
  Dedicated materials show the preview and four map cards in one column;
  the right pane scrolls while the file list remains fixed.
- **Meshes**: open a HIGH, choose its LODs and its physics, check counters,
  dimensions, normals, convex analysis and colours. For the automatic
  associations, set the five suffixes in the import preferences before
  opening the file. Changing a parameter after a quote must force a new
  calculation, without reusing a stale offer.
- **Animations / sounds**: play, stop and reload; for an animation, pause,
  slider, loop and preview on our avatar. The pose must follow the slider
  immediately and stay still during the pause.
  Animation settings sit beside the preview: labelled hand pose and
  expression, aligned loop values in seconds / percent / frames, then easing.
  Sound has a cached waveform, a synchronized playhead, 0–400% import gain
  and start / end crop markers with numeric entry. Committing gain changes
  restarts the audition; crop changes stop it so replay uses the new interval. Reset
  restores the original. The crop and gain affect the uploaded audio and
  are frozen at confirmation; source files and PCM remain unchanged.
  Enable splitting and choose the maximum duration: inspect `_01`, `_02`,
  `_03`, … names, division markers, the shorter tail and the total fee.
- **Multiple / local**: check the total and the confirmation, cancel the
  remaining files, keep the receipts already obtained. For a local mesh,
  apply then remove on a selected object; the removal gives back its
  original appearance. Test a change to the file with watching enabled.

On the grid, the human logs in then validates the price and the debit of
the balance, the folders, the rights and the assets really created for each
type. Also check the server refusal and the resumption of the draft: the
settings stay present, with the warning to check the inventory when the
outcome of a paid transfer is unknown.

The physics uses the free V-HACD 4.1 branch of the Legacy Viewer: resolution in
number of voxels, limit of hulls/vertices, volume error and
Flood/Surface/Raycast modes. The native code is pinned to a public BSD
revision, with cancellation and progress. The model processing is parallel
on the Rust workers, without multiplying the native threads per model.

The draft keeps at most 512 assets and 1 GiB of sources, with 256 files per
selection. The first file beyond the limit produces an explicit error
line; the rest can be added after removing files.

Deliberate difference: a paid transfer whose outcome is unknown is not sent
again automatically. The draft stays available with a request to check the
inventory, to avoid a second payment after the loss of the server's
response. The packets of a same UDP transfer keep their normal
retransmission policy.
