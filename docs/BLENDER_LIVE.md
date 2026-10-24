# LiveMesh

LiveMesh sends rigged mesh edits to your own animated avatar and static
mesh edits to local world objects or attachments in Aurora. You can sculpt,
move vertices, paint weights, edit UVs, adjust shape keys and geometry modifiers,
and see the result without saving or uploading. The preview is local: other
residents do not see it, and it does not create inventory items or charge an
upload fee.

## Static objects (plug-in 0.13.0)

LiveMesh accepts rigged and non-rigged mesh selections, including a mixture
in the same synchronization list. Connect with the selection or use **Add
selected meshes**. A static mesh needs neither an armature nor vertex weights;
shape keys, geometry modifiers, UVs and live materials still work. Geometry is
expressed in metres, retaining authored rotation/scale and ignoring the object's
Blender scene translation; linked children retain their offsets from the root. An existing rig without valid weights remains an
error rather than silently turning into a static object.

With plug-in **0.14.0+**, adding several static meshes at once creates **one
linkset**, even when they have no Blender parents. The active selected static
mesh becomes the **root**; if the active object is rigged, the first static
entry is used. Each mesh keeps its own geometry, materials, relative position,
rotation and size. Rigged meshes stay outside these static linksets.

With plug-in **0.14.1+**, click a name in Blender's synchronization list to select
it. **Ctrl-click** (Cmd-click on macOS) adds or removes individual entries;
**Shift-click** selects a range in the displayed order. **Group** groups only
the selected static entries, using the active selected entry as root.
**Ungroup** on a selected root makes its entire linkset independent. On selected
children, it detaches only those children and keeps the other members linked.
Select any member and click **Make root** to change the root of its complete
linkset. These operations do not change Blender parents or object transforms.
Removing a root from the list promotes a remaining member. Groups survive
save/load, renaming, list reordering and Undo/Redo.

Existing entries without an explicit group still follow Blender parenting:
the highest synchronized static mesh parent is the root, including through
intermediate empties. Explicit grouping overrides this hierarchy; Ungroup also
keeps parented meshes independent in Aurora. The source list groups the prims under one
root, numbered #1, #2 and onward, with an object/prim counter matching the build
tools. Clicking Edit uses the whole linkset by default. Enable **Edit linked parts**
in the normal build tools or object context menu to select a child alone.
All static draw prims, including extra material batches, are direct children
of the same root rather than nested objects. Rez, drop, derez, body/HUD attachment
and detach apply to the complete set, even when
requested from a child. Unparenting inherited groups or using Ungroup creates
independent previews and preserves their current placement. Paused or absent
roots are not referenced. Linksets require the viewer's `object-links` capability.

Static objects initially appear on the floor two metres in front of your
avatar. Select the source in Aurora's **LiveMesh** window to **Edit**, **Drop
to ground**, **Derez**, **Attach to** a body point or **Attach to HUD**. **Detach**
removes a worn preview; **Rez** brings it back. Derez/detach keep the source in
the list and subsequent Blender edits never rez it automatically. The visible
checkbox remains independent. **Stop** clears all session previews and placement.

**Edit** uses the normal Aurora translation, rotation and size tools, with the
same controls for body attachments and HUDs. Right-clicking any prim opens the
ordinary object context menu, with Edit, Edit linked parts, Inspect and supported
local attachment/removal actions.
Inspect shows one object with its separate root and linked prims. Link/unlink
authoring remains in Blender's synchronization list. Numeric
position, rotation and size are also available below the source list; attachment
positions use the attachment point's local frame. Dropping a body attachment
retains its current orientation and horizontal location, resting its rotated
bounds on the supporting floor or terrain below it. Elevated platforms and
skybox floors are picked from scene geometry. Dropping a HUD places it in front of the avatar.

Placement survives renaming, source reordering and live geometry/material
updates. Size edits act as a multiplier over subsequent Blender dimensions.
Render batches for extra materials stay part of one local source. Import/LOD
preparation also accepts static sources, through the existing explicit importer.
The optional `static-meshes` handshake feature protects older viewers: update
Aurora if the plug-in reports it cannot preview non-rigged meshes.

These objects remain local previews visible only in your viewer. They are not
uploaded, saved in grid inventory or simulated by the server; no physics or
script is started. Rigged sources continue to follow the avatar as before.

## Install and connect

1. In Aurora, open **Build → LiveMesh**. The **Plugins** section automatically
   detects local Blender installations and shows each executable's version,
   path and installed Aurora plug-in version. Click **Install**, **Update** or
   **Enable** on the desired row. Aurora chooses the compatible bundled package,
   installs it and saves its enabled state in that Blender profile.
2. If Blender is already open, restart it when convenient to load the updated
   plug-in. Aurora never closes its windows or changes your scene. **Refresh**
   rescans installations. **Choose Blender…** adds an executable in a custom or
   portable location and remembers it for later. Standard application folders,
   common portable folders, PATH and Windows application registrations are
   searched without recursively scanning your disks. Removable drives are added
   through **Choose Blender…**, avoiding empty card readers during automatic
   detection. Prior Aurora package files and `userpref.blend` are preserved under the selected profile's
   `aurora-backups` directory; failed installation restores these files. Newer
   plug-in versions are never downgraded. Multiple executables of the same
   Blender major/minor version can share one profile.

   For installation on another computer or manual control, expand
   **Manual installation (ZIP)** and click **Save Blender plug-in…**. The viewer
   bundles both matching formats; no source checkout, download or external Python
   installation is needed. Install the ZIP matching your Blender version:
   - **Blender 4.2 or later:** choose the **extension** package in Aurora.
     In Blender, open Preferences → Get Extensions → the top-right menu →
     Install from Disk, choose that ZIP, and enable it. Allow its network
     permission: it communicates with `127.0.0.1` only.
   - **Blender 2.93–4.1:** choose the **classic add-on** package in Aurora.
     In Blender, open Preferences → Add-ons → Install, select the ZIP,
     then enable **3D View: Aurora LiveMesh**. Keep the ZIP intact.
3. In Aurora's **LiveMesh** window, click **Enable preview**.
   The default port is **17655**. Change it in both applications if another
   program is already using that port, or to preview in another viewer.
4. In Blender, open the 3D view sidebar (`N`) → **Aurora** → **Live**, then
   **Connect to Aurora**. No object selection or rigged mesh is required.
   To preview meshes, select them and click **Add selected meshes** at any time.
5. Edit your mesh. **Automatic updates** is enabled by default, with an
   **Interval (seconds)** of **0.2**. Larger meshes can take longer to capture
   and prepare. Aurora supplies the
   animation and avatar shape, independently of Blender's current pose.

The four navigation buttons keep the sidebar focused on one task:

- **Live**: connection, automatic/manual update controls and the synchronized-object
  list. **Connection settings** is collapsed by default and holds the port and coordinates.
- **Skeleton**: bone visibility and display options. Expand **Create an SL avatar**
  to create a starter rig and optional test T-shirt.
- **Controls**: hand/foot IK, root/COG controls and independent classic/Bento eye targets.
- **Weights**: separate sections for automatic weights, donor transfer and cleanup.

The **Avatar rig** selector is shared by Skeleton, Controls and Weights. Navigation
belongs to the window manager, stays outside the scene's mesh/settings data,
and defaults to Live on a fresh session.

New starter rigs face **Blender -Y**, like Avastar, with the arms along X.
**Coordinates → Automatic (Avastar / SL)** is the default. It detects each
armature's rest orientation and converts its mesh, normals and joint binds to
Second Life's +X axes together. Imported compatible Avastar rigs do not need
Aurora-specific bone properties. Older Aurora +X rigs and -Y rigs can share one
live list without rotating the Blender objects or editing their weights.
The explicit **SL / Z up** and **Blender -Y forward** choices remain available
and retain their saved values. If an older scene explicitly selected native
coordinates for an Avastar rig, choose **Automatic (Avastar / SL)** and click
**Update now**. Existing scenes and skeletons are never rotated automatically.
See [Avastar's coordinate conventions](https://avalab.org/avastar/300/reference/usermanual/advanced/foreign-rigs/).

**Automatic updates** sends changed geometry, weights, UVs, materials and list
membership at most once per configured interval (**0.1–3,600 seconds**). Idle
scenes send no repeated snapshots, including unchanged texture-paint content.
The interval is expressed in seconds independently of Blender's frame rate.
Uncheck this option to work in **manual mode**: edits remain pending until
you click **Update now**. This button captures immediately while connected,
bypassing the interval and resending even unchanged content. During the
connection handshake, its request waits until Aurora is ready. Connecting
always sends an initial preview, including in manual mode. The interval and
automatic/manual choice are saved per scene in the `.blend` file.

The **Synchronized objects** list is saved with each scene in the `.blend`
file. **Add selected meshes** appends mesh objects without duplicates, keeping
existing entries and their enabled/paused state. You can prepare the list before
connecting. If it is empty, **Connect to Aurora** adds the current mesh selection
for convenience; an existing list is used independently of the current selection.

Each row has a selectable name, a synchronization chain toggle and a remove
button. Selected names are highlighted; the counter below the list shows the
selection size. Turning synchronization off removes the object from the
Aurora preview while retaining its list entry. Turning it back on sends its
latest geometry, weights and materials. Clicking a name only selects entries
for the list actions; it does not change the preview or the scene's 3D selection.
Paused entries can also be selected and grouped. The remove button
removes the entry and its preview, preserving the Blender object. Disabling or
removing every entry sends an empty preview. Add/remove and grouping support Undo/Redo.
Renaming updates the displayed name and keeps the same object reference.
Deleted objects are pruned at the next live tick or list operation.

Disconnecting retains the list and pause flags. Loading another file stops the
connection and restores that file's saved list. During a connection, switching
scenes schedules the new scene's enabled entries according to its update settings.
**Update now** also retries the current enabled objects after an error. List
changes respect the update mode and interval; in manual mode, click **Update now**
to publish additions, pauses, resumptions, removals or grouping changes.

The add-on reconnects automatically when Aurora becomes available again,
resending its latest snapshot. Disconnecting Blender leaves the last good
preview visible. Aurora's **Stop and remove** closes the listener and removes
the preview; **Remove meshes** withdraws all current objects until restored.
Closing the window keeps the bridge active. The listener is
off on each viewer launch, and changing accounts stops it.

## Manage objects and upload in Aurora

Aurora's **Build → LiveMesh** lists each source object by its Blender
name, even if it spans several render batches. The window separates
**Connection**, **Objects** and **Plugins** with horizontal lines and plain headings.
Connection status appears beside the section title; port, loopback address and
the start/stop button share a single line below it.
Uncheck an object to hide it
temporarily; rechecking displays its latest geometry and materials.
**Remove meshes** withdraws all sources from the list and avatar preview.
Subsequent live revisions keep them removed. To bring them back, restart the
preview and use **Update now** in Blender. These controls affect Aurora's
local preview and do not
change the synchronization list or scene in Blender.

Visibility and removal survive source renames with plug-in 0.8.6+ and list
reordering. Settings reset when a source stops being sent (for example, when
disabled/removed in Blender), when stopping the bridge or changing accounts.
Older plug-ins use names as their identity, so renaming starts a new entry.

Each object has its own **Import** button on its row, with no selection required.
With plug-in 0.11.0+, it asks Blender to prepare the current object and all
three lower LODs, then opens them in the normal mesh importer. All four levels
are assigned automatically. All parts, weights, inverse binds, UVs, colors and
image pixels belong to that draft; later live edits cannot alter a pending
quote or upload. Existing drafts remain available, and the new draft is
selected alone. Review the LODs, physics, scale, destination and permissions,
calculate the cost, then confirm the upload normally. Opening the draft
does not request a quote, charge money or create an inventory item.

Select a source in Blender's synchronization list and expand **Import / LOD**.
Each source retains its own Medium, Low and Lowest settings in the `.blend`:

- **Existing object:** choose a mesh explicitly, or detect it by name suffix.
  Defaults are `_medium`, `_low`, `_lowest`, case insensitive. A trailing
  `_high`, `.high` or `-high` is removed before adding the configured suffix.
  Explicit assignment takes priority; hidden objects are supported.
- **Automatic:** reduce disposable copies of the current rest mesh with
  Blender's Decimate modifier. Set a percentage of High triangles (defaults:
  50%, 25%, 12.5%) or an approximate triangle target for each level.
- **Existing or automatic:** the default; use an assigned/detected object,
  generating only when none exists. An invalid existing mesh reports an error
  instead of being silently replaced.

Existing levels must use the same armature and High's materials. Material slots
are matched and reordered; omitted surfaces become empty geometry at that level.
Incompatible or ambiguous slots are rejected. All levels share one validated
joint/bind palette. Generation bakes current shape keys and geometry modifiers
on copies at rest, retaining interpolated weights, UVs and material assignments.
Original meshes, modifiers, keys, pose, selection and list flags stay intact.

In Aurora, select each LOD row to inspect its geometry and triangle count. The
skinned preview's **Test bend (°)** slider bends elbows and knees to inspect
deformations; it changes only the preview pose. Review the reductions visually,
especially seams and small details, before confirming an import. Automatic
triangle counts are approximate and may require adjusting the target.

Preparation waits for Blender's Object Mode, with an actionable error after
30 seconds; Aurora abandons an unanswered request after 180 seconds or a
disconnect. Each bundle is bounded to 600,000 exported corners and 64 MiB.
Older plug-ins can still synchronize live, but must be updated for this import
workflow. A paused source can be imported without enabling its live preview.

The existing mesh upload format includes base colors and diffuse/base-color
textures. PBR material assets require a separate material upload; the importer
displays this limitation beside the frozen-source notice. The live preview
continues to show its supported PBR materials.

Both packages contain the same rig, automatic weights, live materials and
transport. Install only one format in each Blender profile; disable any older
Aurora LiveMesh installation before switching between the two formats.
Updating within the same format replaces the existing add-on. Blender 2.93 LTS
is the minimum supported version; Blender 2.79 is not supported.

Developers can build either format from the repository root:

```powershell
python blender/package.py .scratch/aurora-live-mesh-0.8.7.zip
python blender/package.py --legacy .scratch/aurora-live-mesh-0.8.7-legacy.zip
```

The extension ZIP has a root manifest and targets Blender 4.2+. The classic
ZIP contains an `aurora_live/` directory with `bl_info` and targets 2.93–4.1.

## Create a rig and test T-shirt

In Blender's 3D view, open the sidebar (`N`) → **Aurora** → **Skeleton**,
then expand **Create an SL avatar**. Click
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
clothing. It includes no avatar body mesh. Optional IK controls can be added
through the **Controls** category.

## Skeleton visibility and hand/foot IK

Choose **Avatar rig** in **Skeleton**, or leave it empty to use
the active armature or the active mesh's rig. The category shows separate
toggles for the back/torso, head/neck, arms, legs, Bento fingers, face/eyes, wings, tail, hind
limbs, Fitted Mesh volumes and IK controls. **Show all / Hide all** affect
every bone. Enabling a part also reveals its containing collections/layers,
without revealing unrelated hidden parts. These controls work in Edit Mode
too. **In front**, **Names**, and Blender's rest/pose switch help inspect the
skeleton. Visibility follows the current mode: Object/Pose uses pose visibility,
while Edit Mode uses edit-bone visibility. Visibility changes display only;
they do not disable deformation. Plug-in 0.6.2 corrects Blender 5's separate
pose hide flag, so showing a part and hiding it again both affect the viewport.

### Simple back and advanced Bento spine

In plug-in 0.7.0, **Back / torso** shows the three classic bones: `mPelvis`,
`mTorso` and `mChest`. **Head / neck** has its own toggle. Showing the back
always hides the four folded spine joints, including on existing rigs that
previously showed everything. Hiding the back hides those joints too, so no
overlapping spine bones remain on screen. Other parts keep their visibility.

Expand **Advanced bones → Bento spine** to show `mSpine1` through `mSpine4`
explicitly. This section starts collapsed. **Show all** and the deform-bone
operator still expose the complete rig. Existing scripted `BODY` operator
calls remain supported as classic back plus head/neck.

The apparent three-way split of the lower back is intentional SL geometry.
The [official skeleton XML](https://raw.githubusercontent.com/secondlife/viewer/develop/indra/newview/character/avatar_skeleton.xml)
has this parent chain:

```text
mPelvis → mSpine1 → mSpine2 → mTorso → mSpine3 → mSpine4 → mChest
```

Reading its rest coordinates shows that `mSpine1` runs back down the pelvis
segment and `mSpine2` runs up again over the same 8.4 cm span. This produces
three overlapping Blender bone drawings (`mPelvis` plus those two joints),
rather than three successive lumbar vertebrae. The same arrangement occurs
over `mTorso` with `mSpine3` and `mSpine4`.

The [official Bento spine guide](https://wiki.secondlife.com/wiki/Project_Bento_Skeleton_Guide#Spine)
places two extra joints between pelvis and torso and two between torso and
chest. They permit more flexible custom models. Their folded neutral layout
keeps the classic avatar's proportions, as described in the
[Bento release notes introducing them](https://wiki.secondlife.com/wiki/Release_Notes/Second_Life_Project_Bento/5.0.0.311861).
Custom rigs can deliberately reposition and animate them.
[Avastar's display documentation](https://avalab.org/avastar/300/help/n-panel/avastar/rig-display/)
similarly separates torso visibility from the advanced folded spine.

Aurora changes visibility only: it keeps all 159 deform bones, their parents,
rest matrices, animation and existing weights. Hidden spine weights still
transfer, normalize and export. This display action does not unfold or
reparent joints. Custom joint-position overrides and Blender animation
streaming remain outside the live bridge's current scope.

### Hand/foot IK

In **Controls**, **Create IK controls** adds eight non-deforming target/pole bones
and four independent two-bone IK chains. The 159 SL deformation bones retain
their rest matrices and canonical joint names. The controls are optional,
excluded from live export, and kept in an **Aurora IK** collection (layer 32
in Blender 2.93–3.x). Creating controls again is idempotent. Use a simple,
editable SL armature; limbs already constrained by another rigging tool are
rejected before adding controls.

Plug-in 0.8.5 gives IK controls clean, continuous custom-shape outlines:
rounded hand frames, slim foot platforms with a heel arc, rounded elbow
diamonds and knee circles with small center marks. Left controls are teal;
right controls are coral, with brighter selection states. Decorative cages,
arrows and 3D lettering have been removed. Blender 4.2+ uses a three-pixel
native wire width; earlier versions use Blender's native custom-shape lines.
Shape size follows limb proportions, independent of the tiny underlying
helper bone's length. All eight outlines remain directly selectable.
Hand frames lie flat in the palm's plane, with their long side pointing
toward the fingers, and follow wrist rotation. Earlier vertical Aurora hand
frames migrate automatically without rotating bones or changing a pose.

In **Controls**, adjust **Handle size** (0.4–3 times normal) for the current
rig. **In front** keeps handles visible through the garment. The refresh icon
rebuilds all eight IK widgets without resetting a pose or changing the solver.
Selecting **Target** or **Pole** enters Pose Mode and selects the corresponding
control; you can also click its custom shape directly in the viewport.
The default elbow poles sit behind the arms at elbow height, about two upper
arm lengths away. Each knee pole sits directly ahead of its own knee at knee
height, about 0.6 thigh lengths away. Avatar facing is derived from SL joint
geometry, so the native and -Y starter orientations behave consistently.
The small sideways rest bend of SL legs no longer sends the poles across
the body. When enabling IK on a clearly bent FK limb, its current bend plane
is retained instead.

The solver is initialized geometrically in the pole plane. Shortest-arc bone
rotations retain twist, and the pole angle is measured from the actual seeded
bend, matching Blender's
[ConstrainPoleVector implementation](https://github.com/blender/blender/blob/v4.3.0/intern/iksolver/intern/IK_QJacobianSolver.cpp).
Moving only a foot target upward now bends its knee toward its pole throughout
the lift and return. The original SL neutral leg splay shifts slightly into
that forward plane when IK is enabled (about 2.5 cm at the knee); ankle targets,
SL rest matrices, weights and live export stay fixed. Already-bent FK poses
retain their visible shape when switching to IK.

For an older rig whose active knees still fold sideways, use **Controls >
Feet > Correct knee bend**. It recalibrates only active foot IK chains, using
their existing foot targets and pole positions. Those controls stay fixed
while the knees return to the pole plane. This explicit correction updates
the current pose and leaves animation keys untouched; file loading does not
silently recalibrate existing rigs.

For an existing rig, **Controls > Reposition poles** applies the corrected
placement without rebuilding controls, editing rest bones or changing the
current IK/FK switches. This explicit action affects the current pose; it
does not rewrite animation keys. Loading a file never repositions authored
or animated poles automatically. Avastar's
[IK controls](https://avalab.org/avastar/300/help/n-panel/rigging/ik-controls/)
and [pole-angle explanation](https://avalab.org/avastar/300/help/n-panel/avastar/posing/pose-mode/)
describe the same distinction between endpoint targets and elbow/knee poles.
The native [Blender custom-shape display system](https://docs.blender.org/manual/en/4.2/animation/armatures/bones/properties/display.html)
keeps these shapes outside rendered geometry.

Existing Aurora rigs receive missing/older Aurora widgets on plug-in enable
or file load. Hand-authored custom shapes are preserved during automatic
migration and size changes; the explicit refresh action replaces them.
Widget templates are unlinked datablocks, never scene meshes or synchronized
objects. Existing poses, keyframes, rest bones, weights, constraints and live
capture stay unchanged. Size is saved per armature object. Earlier Blender
uses left/right bone groups for the same colors; 4.0+ uses pose-bone colors.

Enable each hand or foot independently. **Target** selects its control in
Pose Mode: move it to position the wrist/ankle, rotate it to orient the
hand/foot. **Pole** selects the elbow/knee direction control. IK does not
stretch limbs; targets beyond their reach cannot be reached. Enabling snaps
controls to the current FK pose; disabling bakes the visible limb pose back
to FK. The straight SL T-pose uses a tiny local pose seed to avoid Blender's
singular straight-chain solve; the SL rest skeleton stays unchanged. Native
SL joint gaps can introduce pose differences of about one millimetre during switching.
Use Pose Position and leave Edit Mode before switching IK.

These controls support fitting and weight inspection in Blender. Aurora
continues receiving the rest mesh and playing its own avatar animations;
Blender IK poses and animation playback are not streamed. This is a focused
set of SL rigging tools, not an Avastar import/export or animation replacement.

## Root / COG and look-at controls

Plug-in 0.8.0 adds optional pose controls under **Controls** (the former IK
category). They work on existing simple SL rigs as well as the starter.
Create them with the rig outside Edit Mode. Each creation supports Undo,
preserves object selection and the previous mode, and is idempotent. A shared
armature datablock is copied before controls are added. Conflicting bone names
or existing constraints on driven joints are rejected before changing the rig.

### What these helpers represent

Avastar distinguishes its deform skeleton from animator controls. Its
[animation manual](https://deliveries.avalab.org/docs/avastar/300/reference/usermanual/my-first-animation/)
describes **Origin** as scene placement for an entire character, excluded
from its exported animation, and **COG** as movement around the center of
gravity, mapped to the SL pelvis by Avastar's animation exporter.
The [Avastar Eye Rig manual](https://avalab.org/avastar/279/avastar-2/reference/advanced/rigging-eyes/)
describes independent targets for the classic eyes (`mEyeLeft`/`mEyeRight`)
and the alternate Bento eyes (`mFaceEyeAltLeft`/`mFaceEyeAltRight`). Moving a
target rotates the corresponding eyes toward it. A nearby target makes them
converge more strongly. These targets are editing handles, not new SL joints.

### Use Aurora's root and COG

Click **Add root / COG**. **Root** selects the global ring at the armature
origin; move or rotate it to place the whole posed avatar in Blender. **COG**
selects the diamond at the pelvis; it moves the body and rotates it around
that pivot. Active hand/foot IK targets remain anchored relative to Root
when COG moves. Root moves those targets with the character. Both creation
orders (IK first or Root first) work. Existing look targets also follow Root.

The new bones are `auroraPose.Root` and `auroraPose.COG`, both non-deforming.
A pose constraint connects COG to the pelvis without changing any SL rest
matrix or parent. The original pelvis pose/keyframes remain intact. These
are Aurora helpers, not Avastar rig data or the viewer's runtime `mRoot`.

### Use classic and Bento gaze

Under **Look at**, create **Classic eyes +** and/or **Bento eyes +**. A rig
without one of those complete pairs can still use the other. Each target
starts ahead of the eyes at a distance derived from the rig's scale (about
two metres on the standard starter). Focus starts **disabled**, preserving
existing eye poses. Enable its checkbox and choose **Target** to enter Pose
Mode and move the cross-shaped target. Classic and Bento controls are
independent. Disabling focus retains the visible eye rotation for manual
posing. The curved-arrow button recenters the target in front of the current
head pose. The head itself does not automatically turn toward the target.

**Skeleton → Root / COG controls** and **Look-at targets** show/hide these
helpers independently of deform bones and IK controls. Selecting a hidden
helper reveals its group. Custom shapes are unlinked helper datablocks,
not scene meshes. No geometry is added to the synchronized-object list.

All controls stay outside automatic weights, deform-weight transfer/cleanup
and the live joint palette. The existing 159 SL bones, hierarchy and rest
positions remain unchanged. These tools pose the character **in Blender**;
Blender's current Root/COG/eye poses and their animation are not streamed or
exported as SL animation by the live bridge. Aurora keeps animating the rest
mesh itself. Only the supported SL bones should receive garment weights.

## Automatic weight painting

Select your mesh objects and choose **Avatar rig** in **Weights**.
The starter selects its newly created rig automatically. Leaving the field
empty uses each mesh's existing armature, or the single selected armature.
Under **Automatic weights**, click **Recalculate weights** in Object
Mode or Weight Paint mode.
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

## Transfer and normalize weights

In **Weights → Weight transfer**, choose a **Weight source** mesh, then select the
target meshes. The donor can remain selected; it is excluded from targets.
**Transfer weights** replaces only target bone groups, binds the
targets to **Avatar rig** (or their existing/donor rig), keeps the strongest
four weights and normalizes them. **Nearest surface** interpolates weights
on the closest donor triangle; **Nearest vertex** copies the closest donor
vertex's weights. Matching uses world coordinates, including object transforms.
Target matching uses the current shape-key fit, retaining original vertex indices
and excluding target topology modifiers and armature posing.
**Max distance** rejects any vertex too far from the source; zero is unlimited.

The donor is evaluated on a disposable copy, with shape keys and geometry
modifiers but without armature posing. Bone aliases map through `aurora_joint`.
Bento and Fitted Mesh deform weights are included; non-deforming IK controls
are excluded. A donor without deform weights, an unweighted target vertex,
a locked target bone group, or an exceeded distance rejects the entire
selected batch before publishing any weight changes. Unlock target bone
groups deliberately before replacing them.

**Weight cleanup → Normalize to 4 weights** cleans selected meshes' existing deform
weights without a donor. Both operations support Undo and preserve non-bone
mask groups, UVs, shape keys, placement, selection and Weight Paint mode.
Shared target mesh data becomes independent. The donor stays unchanged.
Connected target meshes are added to the live list without resuming paused
entries, and weight changes trigger a new capture.

## Live materials

Plug-in **0.8.5 requires an Aurora viewer with live PBR support**. It recognizes the
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
externally edited image in Blender; **Update now** also refreshes image caches.

## Rig and coordinate conventions

### Send your shape to Blender

With plug-in **0.10.0+** connected, Aurora's LiveMesh **Connection** section
offers **Add a skeleton with my shape** and **Apply my shape to the selected
skeleton**. Aurora requires the worn shape's resolved inventory item
to belong to you and grant **Modify**. Missing inventory data or a no-modify
shape disables the action; Copy/Transfer rights do not substitute for Modify.
Rights, worn item and asset identity are checked again after loading and just
before publishing. A shape change or revoked permission cancels pending work.

The first action creates a **new collection** with the shape's 159-bone SL skeleton and
a neutral rigged system-body reference (head, torso, legs, eyes and eyelashes).
It retains the shape parameters on the new rig. Existing rigs, meshes, weights
and synchronization-list entries remain intact. The reference faces -Y in
Automatic mode and respects scene units and the 3D cursor. Object Mode is
required; an import waiting in another mode expires after 30 seconds. The
creation is undoable. Aurora reports success only after Blender imports it.

The second action requires the target armature to be the active selected object
in Blender (the sidebar's saved rig choice is not a substitute). It keeps the
object, bone names, aliases, hierarchy, constraints, pose channels, weights,
materials and sync-list flags. Generic SL rigs receive the shape's skeletal
proportions; their bound meshes and all shape keys follow that rest change.
Shared mesh/armature data is copied so other objects keep their original data.
Repeating a shape does not multiply its scale. This operation is also undoable.
Unmasked vertex-group Armature modifiers and editable local data are required.
Generic mesh heads/bodies do not acquire surface morphs from SL sliders; the
new reference body has those morphs. An Avastar rig uses the enabled Avastar
add-on's own shape-slider updater for its controls, body morphs and mesh fitting;
with Avastar disabled, it is rejected rather than edited as a generic rig.
Enable Avastar's appearance sliders before applying; its disabled updater is
reported instead of claiming a successful shape change.
Object and Pose Mode are supported for application; Pose Mode is restored after
the operation. Avastar versions without that API report an error. The `shape-apply` capability
is negotiated separately: a 0.9 plug-in can create references but cannot receive
an Apply command and accidentally create another rig.

This transfers the saved worn **shape**, not worn mesh bodies, heads, clothes,
skin textures, animations or unsaved shape-editor changes. Use the reference
to fit a garment or transfer its weights. It is not added to the live list
automatically. Shape-specific bind matrices keep a garment rigged to this new
skeleton from receiving the shape twice when previewed in Aurora.

Use a Second Life compatible armature and its deform vertex groups (`mPelvis`,
`mTorso`, `mChest`, Bento joints, supported aliases and collision volumes).
An optional string custom property **`aurora_joint`** on a Blender bone maps
its name to an Aurora joint. Unrecognized weighted bones are reported instead
of silently attaching the mesh to the pelvis. Every vertex needs a positive
bone weight. The four strongest influences are retained and normalized, as
in Second Life mesh assets.

**Automatic (Avastar / SL)** detects an existing SL rig's rest axes. Explicit
**SL / Z up** and **Blender -Y forward** overrides remain available. Scene unit scale is
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
& '<Blender executable>' --background --factory-startup --python-exit-code 1 --python blender/tests/test_rig_tools.py
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
translation removed), `features: ["pbr-materials", "source-ids", "shape-transfer", "shape-apply", "import-lods"]`, and `max_bytes`. Every subsequent client
message is a four-byte unsigned little-endian JSON byte count, then a UTF-8
snapshot. Length zero is a heartbeat. The reply is newline-terminated JSON
with `ok`, `revision` on success, or `error` on rejection. A success acknowledges
CPU preparation, not a displayed GPU frame. Revisions strictly increase within
a connection; reconnecting starts a new revision sequence.
Only an explicitly transient staging failure sets `retry: true` on a rejected
reply; the client retries its newest snapshot after 0.5 seconds without needing
another edit. Invalid content is retained as an error until edited or resent.

Shape-capable clients explicitly negotiate with a framed
`{"type":"features","features":["shape-transfer"]}` control. An explicitly
authorized shape may then accompany one normal reply in `shape`, containing
a transient request `id`, display name, shape parameters, canonical rest
joints/inverse binds and neutral system-body geometry (maximum 8 MiB).
The main-thread import answers with
`{"type":"shape-result","id":"<request UUID>","error":null}` or an error.
Controls are bounded to 4 KiB and never affect mesh revisions. Old clients
receive no shape data. Pending sends are cleared on disconnect, not replayed.
An optional `shape_cancel` request UUID in a reply clears a matching reference
still waiting for Object Mode when authorization is revoked or the send expires.

Clients negotiating `import-lods` can receive
`prepare_import: {"id":"<request UUID>","source":"<source UUID>"}` in a reply.
Blender's main thread prepares the source's configured levels; its socket worker
sends a length-prefixed body beginning with four ASCII bytes `LODS`, followed
by UTF-8 JSON containing `id`, `source`, `levels` and shared `images`. `levels`
is ordered Lowest, Low, Medium, High. Each level contains the regular wire
`mesh`, `slots` mapping its nonempty surfaces to High, and a bounded `origin`
label. Errors send `id`, `source`, `error` instead. The viewer validates the
bundle on its transport worker and reuses the strict mesh decoder; it never
places these scenes in the live render mailbox. Request identity is checked
before decoding and again before publication. Stale responses are ignored;
requests are transient, cancelled on disconnect and never replayed. LOD replies
and acknowledgements do not advance or replace live mesh revisions.

Snapshots have `protocol`, `revision` and `meshes`. Each mesh has `name`,
`joints`, column-major `inverse_bind` matrices, and `faces`. The optional
`static-meshes` capability permits empty `joints` and `inverse_bind` arrays;
all static vertex joint indices and weights must be zero. A nonempty joint
palette still requires valid positive weights. The optional
`source_id` is a non-null UUID unique within a snapshot, identifying a scene's
synchronization entry, never a grid asset. The add-on sends it only when the
viewer advertises `source-ids`; older viewers continue to receive v1 meshes
without that field. Older add-ons work with name-based identity.
The optional `object-links` feature adds `link: {parent, offset}` to a static
mesh. `parent` is a synchronized, unskinned root source UUID; `offset` is the
child origin minus root origin, in metres in the same baked coordinate frame
as its vertices. Each source keeps its own geometry and identity. Roots have
no link, and the exporter flattens explicit LiveMesh groups and inherited
Blender parenting to the same format. Missing, cyclic, rigged,
self and excessive-offset links are rejected before installation. The worker
orders roots before children independently of the synchronization-list order.
A peer without this feature receives a clear update error for linked scenes.

A face contains
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
