# Blender Live Mesh

Blender Live sends your rigged mesh edits to your own animated avatar in
Aurora. You can sculpt, move vertices, paint weights, edit UVs, adjust shape
keys and geometry modifiers, and see the result without saving or uploading.
The preview is local: other residents do not see it, and it does not create
inventory items or charge an upload fee.

## Install and connect

1. In Aurora, open **Build → Blender Live** and click **Save Blender
   plug-in…**. Choose where to save the ZIP. The viewer bundles its matching
   plug-in; no source checkout or external Python installation is needed.
2. In **Blender 4.3 or later**, open Preferences → Get Extensions → the
   top-right menu → Install from Disk, choose that ZIP, and enable it.
   Allow its network permission: it communicates with `127.0.0.1` only.
3. In Aurora's **Blender Live** window, click **Enable preview**.
   The default port is **17655**. Change it in both applications if another
   program is already using that port, or to preview in another viewer.
4. In Blender, select the rigged mesh objects, open the 3D view sidebar
   (`N`) → **Aurora**, and click **Connect to Aurora**.
5. Edit your mesh. The default capture interval is **0.2 seconds**; larger
   meshes can take longer to capture and prepare. Aurora supplies the
   animation and avatar shape, independently of Blender's current pose.

The selection is captured when you connect. **Use selected meshes** changes
the tracked set; an empty selection removes the preview. **Send now** retries
the current mesh after an error. Deleting a tracked object removes its preview
at the next capture. Renaming does not lose the tracked object.

The add-on reconnects automatically when Aurora becomes available again,
resending its latest snapshot. Disconnecting Blender leaves the last good
preview visible. Aurora's **Stop and remove** closes the listener and removes
the preview; **Remove meshes** clears the current preview until Blender next
sends a change. Closing the window keeps the bridge active. The listener is
off on each viewer launch, and changing accounts stops it.

Developers can also build the same extension from the repository root:
`python blender/package.py .scratch/aurora-live-mesh-0.2.0.zip`.
Blender 4.3 is the minimum supported version.

## Create a rig and test T-shirt

In Blender's 3D view, open the sidebar (`N`) → **Aurora**. Click
**Create SL avatar rig** to add a new armature at the 3D cursor. Leave
**Add rigged test T-shirt** enabled to create a garment with it, or disable
the option for the armature alone. Use Object Mode to create a starter.
The operator supports Undo and never replaces existing scene objects.

The rig contains **all 133 Second Life Bento bones and 26 Fitted Mesh
collision bones**, using the same `avatar_skeleton.xml` as Aurora. Original
joint names, parents and rest positions are preserved. Bone collections
separate the body, hands, face, wings, tail and other extensions. Body and
hand collections are visible initially; enable the other collections in
Armature Data Properties when needed. All 159 bones can deform meshes.

The optional T-shirt has editable quad topology, open neck/hem/cuffs, a UV
map, a fabric material, an Armature modifier and normalized vertex weights
(up to four influences). It is selected when created: click **Connect to
Aurora** to preview it. If already connected, creating a T-shirt switches
the tracked mesh to the new shirt. Pose the armature to inspect its weights
in Blender; Aurora still receives the rest mesh and uses its own animations.

The starter respects the current **Coordinates** choice and scene unit
scale. Its proportions are the standard SL rest skeleton, independent of
your current account's shape. The shirt is sample geometry for testing
weights and live edits; fit it to your intended body before using it as
clothing. It includes no avatar body mesh or animation control/IK rig.

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

- 32 source objects, 600,000 exported corners, 64 MiB per snapshot.
- 110 weighted joints per source object and 32 nonempty material surfaces.
  Large surfaces and more than eight faces are split into render objects.
- Material base colors and alpha are live. Texture images, shader graphs,
  metallic/roughness maps and Blender animation playback are not transmitted.
- This previews the fit over the avatar and its existing outfit. It does not
  replace an inventory attachment or automatically hide body/outfit parts.
- Saving the `.blend` file does not save the connection. File loading and
  disabling the extension stop its worker. UVs travel with the mesh, although
  material colors alone cannot show a UV layout without textures.

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

## Wire protocol v1

TCP listens on IPv4 loopback only. The client sends the eight bytes
`AURBLD1\n`. Aurora responds with one newline-terminated JSON object containing
`protocol`, the supported `joints`, their inverse neutral `rest_axes` (column-major,
translation removed), and `max_bytes`. Every subsequent client
message is a four-byte unsigned little-endian JSON byte count, then a UTF-8
snapshot. Length zero is a heartbeat. The reply is newline-terminated JSON
with `ok`, `revision` on success, or `error` on rejection. A success acknowledges
CPU preparation, not a displayed GPU frame. Revisions strictly increase within
a connection; reconnecting starts a new revision sequence.

Snapshots have `protocol`, `revision` and `meshes`. Each mesh has `name`,
`joints`, column-major `inverse_bind` matrices, and `faces`. A face contains
linear RGBA `color`, `vertices` and triangle `indices`; a vertex has `position`,
`normal`, `uv`, four `joints` indices and four `weights`. `meshes: []` removes
all previews. No paths, executable commands, grid URLs or asset UUIDs are
accepted. The socket thread parses and encodes meshes and publishes one
replaceable latest snapshot; Blender's socket thread also uses a single
latest snapshot and never accesses Blender data.

Blender API references: [evaluated dependency graph](https://docs.blender.org/api/4.3/bpy.types.Depsgraph.html),
[object mesh capture](https://docs.blender.org/api/4.3/bpy.types.Object.html#bpy.types.Object.to_mesh),
[mesh corner normals and triangulation](https://docs.blender.org/api/4.3/bpy.types.Mesh.html).
