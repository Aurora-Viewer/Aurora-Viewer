# Blender Live Mesh

Blender Live sends your rigged mesh edits to your own animated avatar in
Aurora. You can sculpt, move vertices, paint weights, edit UVs, adjust shape
keys and geometry modifiers, and see the result without saving or uploading.
The preview is local: other residents do not see it, and it does not create
inventory items or charge an upload fee.

## Install and connect

1. In Aurora, open **Build → Blender Live**, choose your Blender version,
   and click **Save Blender plug-in…**. Choose where to save the ZIP. The
   viewer bundles both matching formats; no source checkout or external
   Python installation is needed.
2. Install the ZIP that matches your Blender version:
   - **Blender 4.2 or later:** choose the **extension** package in Aurora.
     In Blender, open Preferences → Get Extensions → the top-right menu →
     Install from Disk, choose that ZIP, and enable it. Allow its network
     permission: it communicates with `127.0.0.1` only.
   - **Blender 2.93–4.1:** choose the **classic add-on** package in Aurora.
     In Blender, open Preferences → Add-ons → Install, select the ZIP,
     then enable **3D View: Aurora Live Mesh**. Keep the ZIP intact.
3. In Aurora's **Blender Live** window, click **Enable preview**.
   The default port is **17655**. Change it in both applications if another
   program is already using that port, or to preview in another viewer.
4. In Blender, select the rigged mesh objects, open the 3D view sidebar
   (`N`) → **Aurora**, click **Add selected meshes**, then **Connect to Aurora**.
5. Edit your mesh. The default capture interval is **0.2 seconds**; larger
   meshes can take longer to capture and prepare. Aurora supplies the
   animation and avatar shape, independently of Blender's current pose.

The **Synchronized objects** list is saved with each scene in the `.blend`
file. **Add selected meshes** appends mesh objects without duplicates, keeping
existing entries and their enabled/paused state. You can prepare the list before
connecting. If it is empty, **Connect to Aurora** adds the current mesh selection
for convenience; an existing list is used independently of the current selection.

Each row has a synchronization checkbox and a remove button. Unchecking an
object removes it from the Aurora preview while retaining its list entry.
Rechecking sends its latest geometry, weights and materials. The remove button
removes the entry and its preview, preserving the Blender object. Disabling or
removing every entry sends an empty preview. Add/remove support Undo/Redo.
Renaming updates the displayed name and keeps the same object reference.
Deleted objects are pruned at the next live tick or list operation.

Disconnecting retains the list and pause flags. Loading another file stops the
connection and restores that file's saved list. During a connection, switching
scenes synchronizes the new scene's enabled entries. **Send now** retries the
current enabled objects after an error.

The add-on reconnects automatically when Aurora becomes available again,
resending its latest snapshot. Disconnecting Blender leaves the last good
preview visible. Aurora's **Stop and remove** closes the listener and removes
the preview; **Remove meshes** clears the current preview until Blender next
sends a change. Closing the window keeps the bridge active. The listener is
off on each viewer launch, and changing accounts stops it.

Both packages contain the same rig, automatic weights, live materials and
transport. Install only one format in each Blender profile; disable any older
Aurora Live Mesh installation before switching between the two formats.
Updating within the same format replaces the existing add-on. Blender 2.93 LTS
is the minimum supported version; Blender 2.79 is not supported.

Developers can build either format from the repository root:

```powershell
python blender/package.py .scratch/aurora-live-mesh-0.5.0.zip
python blender/package.py --legacy .scratch/aurora-live-mesh-0.5.0-legacy.zip
```

The extension ZIP has a root manifest and targets Blender 4.2+. The classic
ZIP contains an `aurora_live/` directory with `bl_info` and targets 2.93–4.1.

## Create a rig and test T-shirt

In Blender's 3D view, open the sidebar (`N`) → **Aurora**. Click
**Create SL avatar rig** to add a new armature at the 3D cursor. Leave
**Add rigged test T-shirt** enabled to create a garment with it, or disable
the option for the armature alone. Use Object Mode to create a starter.
The operator supports Undo and never replaces existing scene objects.

The rig contains **all 133 Second Life Bento bones and 26 Fitted Mesh
collision bones**, using the same `avatar_skeleton.xml` as Aurora. Original
joint names, parents and rest positions are preserved. Bone collections
separate the body, hands, face, wings, tail and other extensions in Blender
4.0+. Blender 2.93–3.x uses matching named pose groups and one armature layer
per group. Body and hand sets are visible initially; enable other collections
or layers in Armature Data Properties when needed. All 159 bones can deform
meshes. Classic layer indices are recorded in the rig's
`aurora_bone_layers` custom property.

The optional T-shirt has editable quad topology, open neck/hem/cuffs, a UV
map, a fabric material, an Armature modifier and normalized vertex weights
(up to four influences). It is selected when created: click **Connect to
Aurora** to preview it. Creating a T-shirt adds it to the synchronization list,
keeping existing entries, whether or not a connection is active. Pose the
armature to inspect its weights in Blender; Aurora still receives the rest
mesh and uses its own animations.

The starter respects the current **Coordinates** choice and scene unit
scale. Its proportions are the standard SL rest skeleton, independent of
your current account's shape. The shirt is sample geometry for testing
weights and live edits; fit it to your intended body before using it as
clothing. It includes no avatar body mesh or animation control/IK rig.

## Automatic weight painting

Select your mesh objects and choose **Avatar rig** in Sidebar → Aurora.
The starter selects its newly created rig automatically. Leaving the field
empty uses each mesh's existing armature, or the single selected armature.
Click **Recalculate automatic weights** in Object Mode or Weight Paint mode.
This replaces the selected meshes' bone weights using Blender's native heat
solver, keeps the strongest four influences and normalizes them. Undo restores
the previous result. An existing Aurora connection immediately tracks these
meshes and sends the updated weights. Existing list entries remain; paused
objects keep their pause state.

**SL body** uses enabled classic body deform bones, excluding overlapping
Bento and collision bones. **All deform bones** includes enabled Bento/custom
deform bones, except collision volumes and the four overlapping Bento spine
joints. The source rig's deform flags and rest pose are preserved. Empty
counterpart groups are kept for Mirror modifiers. Non-bone groups, shape keys,
UVs, selection and world placement are preserved; a mesh shared with another
object becomes independent before its weights change.

The calculation runs on disposable mesh/rig copies in rest pose. If a solve
leaves any vertex unweighted, no selected source mesh is changed. Check the
topology and retry, or paint the problematic weights manually. Use editable
objects with at most one Armature modifier.

## Live materials

Plug-in **0.5.0 requires an Aurora viewer with live PBR support**. It recognizes the
connected Principled BSDF of each material: base color, alpha, metallic,
roughness, emission color/strength and backface culling. Older Principled
input names and Separate RGB channel nodes are supported. Direct RGB/Value
constants work too. Changing material parameters or assignments updates the
preview without moving the mesh. Texture painting also captures continuously
at the normal update interval.

The supported image maps are base color, tangent-space normal, metallic/
roughness and emission. Images are embedded from Blender's loaded pixel data;
no paths are sent to Aurora. Metallic/roughness channels are packed automatically.
Use a Normal Map node in Tangent space with strength 1; connect an image's Alpha
output to Principled Alpha to use its transparency. Otherwise image alpha is
ignored, as in Blender. Alpha is opaque or blended.

Use the mesh's active UV map, flat Image Texture nodes, Repeat extension and
Linear interpolation. A Point Mapping node supports translation, scale and Z
rotation. Metallic and roughness maps must share the same UV transform. Image
color spaces are sRGB, Linear Rec.709/Linear or Non-Color. Each source image is
limited to 16 megapixels and downsampled to at most 1024 pixels on its longest
edge. At most 16 images and 16 MiB of RGBA pixels travel in one snapshot.
Unchanged images reuse GPU slots; old materials/images remain until their last
geometry reference is released. Unchanged image mip chains/staging are reused
on the socket worker too; a temporary staging shortage retries automatically.

Bake procedural shaders, material groups and advanced Principled effects
(coat, transmission, subsurface, sheen) to compatible images first. SL's PBR
model does not represent every Blender shader input. The plug-in reports
unsupported connected inputs and keeps the last valid preview. Reload an
externally edited image in Blender; **Send now** also refreshes image caches.

## Rig and coordinate conventions

Use a Second Life compatible armature and its deform vertex groups (`mPelvis`,
`mTorso`, `mChest`, Bento joints, supported aliases and collision volumes).
An optional string custom property **`aurora_joint`** on a Blender bone maps
its name to an Aurora joint. Unrecognized weighted bones are reported instead
of silently attaching the mesh to the pelvis. Every vertex needs a positive
bone weight. The four strongest influences are retained and normalized, as
in Second Life mesh assets.

**SL / Z up** is the default for an existing SL rig. If your rig faces Blender's
negative Y direction, choose **Blender -Y forward**. Scene unit scale is
converted to metres. The armature's placement and orientation in the Blender
scene are removed; its authored scale, the mesh's relative transform, normals,
mirrored winding, UV seams and material assignments are preserved.

Inverse binds use source rest joint heads in the SL coordinate frame and
Aurora's neutral joint axes, including collision-volume rotation and scale. Blender
bone roll is a rigging aid, not an SL joint rest rotation. The current Blender
pose is excluded: capture evaluates temporary mesh copies with Armature
modifiers removed, keeping geometry modifiers and shape keys. The source mesh,
selection, object mode and modifier settings remain intact. Custom joint
position overrides are not applied to Aurora's skeleton.

## Current limits

- 32 enabled source objects, 600,000 exported corners, 64 MiB per snapshot.
  Paused entries remain in the list and do not count toward these limits.
- 110 weighted joints per source object and 32 nonempty material surfaces.
  Large surfaces and more than eight faces are split into render objects.
- Materials follow the supported PBR/image conventions above. Arbitrary
  Blender shader graphs and Blender animation playback are not transmitted.
- This previews the fit over the avatar and its existing outfit. It does not
  replace an inventory attachment or automatically hide body/outfit parts.
- Saving the `.blend` file does not save the connection. File loading and
  disabling the extension stop its worker.

Aurora retains the last valid geometry while a replacement is prepared.
Invalid snapshots are rejected as a whole. Obsolete preview GPU buffers are
freed when no draw object refers to them. Render objects occupy a private
namespace and cannot be selected for simulator edits or touches.
They are excluded from automatic hover calibration, so fitting preview shoes
cannot change the hover height sent to the simulator.

## Offline verification

Run the normal `scripts/check.ps1` for protocol, skinning and socket tests.
The exporter tests use Blender itself without logging into a grid:

```powershell
& '<Blender executable>' --background --factory-startup --python-exit-code 1 --python blender/tests/test_exporter.py
& '<Blender executable>' --background --factory-startup --python-exit-code 1 --python blender/tests/test_starter.py
& '<Blender executable>' --background --factory-startup --python-exit-code 1 --python blender/tests/test_materials_weights.py
& '<Blender executable>' --background --factory-startup --python-exit-code 1 --python blender/tests/test_tracking.py
python -m unittest discover -s blender/tests -p 'test_client.py' -v
```

`AURORA_DEMO=1` plus `AURORA_DEMO_BLENDER_LIVE=1` opens the bridge window and
places a purple rigged shell on the demo avatar. It uses the production
protocol and skinning pipeline. A Blender client can replace it live.

With that offline viewer running, `blender/tests/bridge_smoke.py` (run with
Blender's same background arguments) sends a real exported mesh, edits it,
checks a rejected weight, clears it and restores a final green mesh. Pass
`-- --port <port>` after the script path when using another local port.
Add `--starter` to use the generated 159-bone rig and weighted T-shirt
instead of the sphere, including its actual inverse binds and weight palette.
Add `--pbr` to recalculate its automatic weights, send base/normal/ORM/emission
maps and stream repeated image edits through the renderer.
`bridge_tracking.py` uses the real Blender client and list operators to add two
objects, pause them individually/together, resume updated pixels, remove/re-add
entries and reconnect without a selection. Use the same `-- --port <port>`
arguments. Its optional `--save-blend <absolute-path.blend>` saves the final list
with one active object, one paused object and a packed test image.
Add `--save-blend <absolute-path.blend>` with `--pbr` to save the prepared
rig, weighted T-shirt and packed test images before connecting. The saved file
can be opened normally and connected through the Aurora sidebar.

## Wire protocol v1

TCP listens on IPv4 loopback only. The client sends the eight bytes
`AURBLD1\n`. Aurora responds with one newline-terminated JSON object containing
`protocol`, the supported `joints`, their inverse neutral `rest_axes` (column-major,
translation removed), `features: ["pbr-materials"]`, and `max_bytes`. Every subsequent client
message is a four-byte unsigned little-endian JSON byte count, then a UTF-8
snapshot. Length zero is a heartbeat. The reply is newline-terminated JSON
with `ok`, `revision` on success, or `error` on rejection. A success acknowledges
CPU preparation, not a displayed GPU frame. Revisions strictly increase within
a connection; reconnecting starts a new revision sequence.
Only an explicitly transient staging failure sets `retry: true` on a rejected
reply; the client retries its newest snapshot after 0.5 seconds without needing
another edit. Invalid content is retained as an error until edited or resent.

Snapshots have `protocol`, `revision` and `meshes`. Each mesh has `name`,
`joints`, column-major `inverse_bind` matrices, and `faces`. A face contains
linear RGBA `color`, `vertices` and triangle `indices`; a vertex has `position`,
`normal`, `uv`, four `joints` indices and four `weights`. `meshes: []` removes
all previews. No paths, executable commands, grid URLs or asset UUIDs are
accepted. The socket thread parses and encodes meshes and publishes one
replaceable latest snapshot; Blender's socket thread also uses a single
latest snapshot and never accesses Blender data.

An optional root `images` array holds `width`, `height` and Base64 `rgba`
(RGBA8, top row first). A surface's optional `material` contains `metallic`,
`roughness`, linear `emissive`, `double_sided`, `alpha_mode` (`OPAQUE`, `BLEND`,
`MASK`), four `textures` indices (base, normal, ORM, emission; null if absent)
and four UV `transforms` ([offset U/V, scale U/V, rotation radians]). Surface
`color` is the linear base factor. Color images use sRGB, data images linear.
Missing material/image fields preserve the original v1 color-only behavior.

Blender API references: [evaluated dependency graph](https://docs.blender.org/api/4.3/bpy.types.Depsgraph.html),
[object mesh capture](https://docs.blender.org/api/4.3/bpy.types.Object.html#bpy.types.Object.to_mesh),
[mesh corner normals and triangulation](https://docs.blender.org/api/4.3/bpy.types.Mesh.html).
