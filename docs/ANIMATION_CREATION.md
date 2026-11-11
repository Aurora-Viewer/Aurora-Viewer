# Create native animations

Open **Build > IK Edit > Animations**, then **Create an animation**. The
creation mode uses the same skeleton, move/rotation tools,
world/local gizmo, Ctrl rotation, symmetry and hand/foot IK handles.
The Animation editor has its own window: the animation's name with the
drafts beside it, then four pages:

- **Bone**: a clickable skeleton to pick a bone, then the selected bone's
  IK/FK mode, position and rotation, its track's interpolation and priority,
  and the resets. The constraint switch and strength, orientation, contact
  point, limb constraints, world/local gizmo space and symmetry are in its
  folded **Advanced** section.
- **Tracks**: every keyed bone with its switch, priority and interpolation.
- **Clip**: range, FPS and the playback settings.
- **Display**: which bone groups show their handles.

Playback, scrubbing and key navigation are in the timeline window, which
opens with the editor; once closed, **Timeline** in the editor reopens it.
**Return to IK contacts** restores the existing contact definitions and
their enable states (it is not offered when the editor was opened from
Avatar Studio).

## The timeline window

One toolbar sits above the tracks and stays in place while they scroll:

- **Transport**: rewind (Home), previous key, play/pause (Space), next key
  and loop. The loop range (full timeline or last keyframe) shows while
  looping.
- **Tools**: Move or Rotate, **IK** or **FK** for the selected bone (IK moves
  its chain, FK edits the joint alone), **Mirror** for symmetric edits, and the
  hand/foot limb handles.
- **Keying**: **Key** records the selected bone (K); auto-key shows its step
  while it is on.
- **Edit**: undo, redo, copy pose, paste pose, delete the selected bone's key
  (Delete).
- **View**: zoom out, zoom in, fit. Ctrl+wheel zooms around the frame under the
  pointer, Shift+wheel pans, the plain wheel scrolls the bone list.
- **File**: import an animation, open or save an editor project (Ctrl+S), and
  delete every key of the current frame.

The frame ruler shares the lanes' scale: click or drag it to scrub. Left and
Right step one frame, and keep stepping while held. Keys are diamonds on each
bone's lane: drag one to move it, click an empty spot to go to that frame. A
key belongs to the frame it is drawn on, including keys of imported animations
that fall between frames; moving or re-recording such a key snaps it to the
frame. Copy pose takes the keyed bones only, so pasting never keys the bones
you did not touch.

## Work in Avatar Studio

Open **Avatar Studio** from its toolbar button or the Build menu, then choose
**Animate** in the Studio controls. This opens the same native animation
editor over the Studio avatar; the existing orbit, pan, focus and zoom controls
remain available. Create an animation or resume the current draft in the
Animations tab. Pose with the existing bone handles, then use **Key this bone**,
**Key changes**, **Key full pose**, or auto-key to record the edit. Auto-key
records when you finish moving a bone, then advances the playhead by the chosen
step (0.5 seconds by default) so you can edit the next pose without selecting a
new time first. Adjust the step beside the toggle, or turn auto-key off for
manual key placement. The timeline is a movable, edge-resizable window that
opens at the lower right beside the editor when the workspace allows it and
remembers its position and size. Its track list
scrolls independently when an animation has more bones than fit in the window.
Close it from the title bar and reopen it with **Timeline** in the Studio
controls. The timeline provides scrubbing, playback, loop preview, undo/redo,
copy/paste and key deletion. Choose whether
preview looping covers the full timeline or stops at the last key in an enabled
authored track; the animation's exported loop setting remains on the Clip
page. Bone markers and manipulators hide during playback and return when
you pause, leaving a clean preview without changing the Display visibility
settings.
The local avatar's nameplate is hidden while Animate is active. **Hover Height**
in the Studio toolbar moves only the displayed avatar in 1 cm increments; it
does not change the animation or world position. Its fixed-width toolbar button
and wrapped, padded slider panel stay in place while the value changes.
Zoom and pan there to work on closely spaced keys in a long clip, or choose
**Fit** to see the full range. Use **Save editor project** to preserve an
editable draft; **Save .anim** and **Prepare upload** produce the native clip.
When **mPelvis** is selected in the Animation Editor, **Root height** on the
Bone page adjusts
its vertical translation relative to the avatar's rest position. Key the
takeoff, apex and landing at separate times to create a jump; the pelvis
translation carries the avatar's skeleton in preview and in the exported
animation. It does not move the avatar's simulator position. The selected
bone's Avatar Frame values stay stable while paused and refresh on selection,
scrubbing or playback, instead of tracking unrelated live idle motion.
In animation mode, the Bone page exposes rotation and position only because native
Second Life animation files cannot store joint scale. Entering the editor
temporarily switches a scale-mode Poser to rotation and restores the selected
Poser tool when you return to posing.
The Poser settings' **Posing sensitivity** control scales the move, rotation and
scale gizmos as well as the trackball. Its default of 0.5 keeps the existing
drag response; lower it for smaller changes per drag, and the choice is saved
with the other Poser settings.

## Record keys

The initial frame records the rest skeleton with your IK contacts solved on
it. An animation is built from there: what plays on the avatar is not part of
it, and while creating, the avatar shows the rest skeleton and your keys. Manual
joint edits made in the contact editor stay there; pose them here. Choose a
frame in the timeline, pick a bone, pose the avatar, then record it:

- **Key this bone** records the selected joint. When its IK chain is being
  edited, the affected ancestors are recorded too; symmetry includes the
  edited counterpart.
- **Key changes** records the joints affected by the staged pose edits.
- **Key full pose** records the real skeletal joints in the displayed pose.

Recording is applied after the pose has been evaluated. The temporary pose
correction is removed when its new sampled keys are ready, so influence
weights are baked once. Other unrecorded pose edits remain staged. Seeking
or playing clears these temporary corrections and displays the keyed clip.
Key navigation and deletion are in the timeline window.

A key belongs to the frame the timeline shows it on, even when an imported
`.anim`/`.bvh` file or a change of FPS left it between two frames: deleting,
dragging or recording at that frame acts on it, and a moved or re-recorded
key lands on the frame's exact time, one key per frame. **Copy pose** copies
the authored, enabled tracks only, so pasting a pose never adds an untouched
bone to the exported animation; deleting a frame's keys leaves the hidden
rest-pose baseline of a fresh draft in place.

## Pick a bone

The Bone page's skeleton is drawn from the avatar's own rest skeleton, seen
from the front (its left side on the right, marked R and L). Choose **Body**
for the pelvis, spine, neck, head, arms and legs, **Hands** for the finger
joints of both hands (seen from above, each hand in its own half) or
**Face** for the face joints and eyes. Hover a joint to enlarge it and see
its name, click it to select the bone; the selected bone is teal and a
violet ring marks the bones keyed at the current frame. Picking a bone of a
hidden group shows that group's handles.

In the Avatar Studio timeline, zoom in around the current frame to place or
move closely spaced keys, pan across long clips, and use **Fit** to return to
the full animation range. A vertical playhead line crosses every bone track,
and playback follows it while zoomed in. Hover a key marker to highlight it,
see its frame number, and get a grab cursor; the cursor changes while dragging.

Rotations are native local quaternions. Pelvis translations are relative to
the avatar root. Other joint positions are included when a direct edit
changes them; ordinary rotation keys retain the avatar's joint positions.
The authoring grid is **30 fps**, from frame 0 to 1800 (60 seconds). Native
files contain continuous key times rather than an original frame rate.
Position channels must stay within the native format's -5 to +5 metre range
per axis. An invalid capture reports an error without writing a partial key.

## Set priorities and playback

**Tracks** enables each track and sets its priority independently; the Bone
page sets the selected bone's priority and interpolation too.
`-1` inherits the global animation priority. Disabling a track excludes it
from preview/export while keeping its keys in the draft. Turning off the final
track shows the rest skeleton; exporting still requires an active track with
keys.

Fresh drafts seed the rest skeleton for an isolated preview, but those baseline
tracks are not exported until you key or otherwise edit them. This keeps a
single-joint animation, such as a wave, from locking every untouched joint.
Imported animation tracks remain authored and export as before.

**Clip** contains:

- Start/end frames and the resulting exported duration. Trimming retains
  outside keys in the draft, samples both new boundaries, and rebases the
  exported file to time zero.
- Global priority, loop enable, loop start/end, ease-in and ease-out.
- Native hand pose and expression.

Loop times are seconds relative to the exported start. The creation preview
shows exact keyed frames; the ordinary import preview displays final
playback easing, hand pose and expression. The existing native codec writes
the file and validates the complete result before saving or preparing import.

## Reuse an IK pose and keep drafts

Return to IK contacts to adjust prim targets, bone targets or virtual
controls. In Animations, choose the destination **Frame (.anim)** and
**Capture current IK pose** to add your contacts, solved on the rest skeleton,
to the current draft. This bakes the real skeletal pose; the LSL companion
supplies live target following during play.

Creating another animation retains the previous draft in the draft selector.
Up to sixteen drafts remain available during the viewer session, including
after closing/reopening IK Edit. Save `.anim` files to keep exported clips
before leaving the viewer. Outside trimmed keys and disabled tracks remain
in the session draft; an exported file contains the selected active clip.
Editable `.auroraanim` projects preserve the full timeline and keys, use a
64 MiB file-size limit, and reject an oversized save rather than writing a
project that cannot be reopened.
Opening `.anim`, `.bvh` or `.auroraanim` files reads at most 64 MiB before
decoding, matching the parser limits.

## Save or import

**Save .anim** opens a native file dialog and writes the validated file.
Stepped tracks use adjacent native time buckets at each change so exported
playback holds the pose and changes at the same point as the editor preview.
Smooth and Bézier tracks are sampled at up to 60 Hz, independently of the
timeline FPS, while authored key times are retained. The exporter reduces its
extra samples only when needed to fit the native per-joint key limit. This
keeps low-FPS timelines from flattening eased motion in the exported animation.
The editor's BVH import rejects files containing joints that are not mapped
to the current avatar rig, avoiding a partial animation; the general BVH
importer keeps its existing behavior. Project files with unknown or duplicate
bone tracks are rejected instead of opening an ambiguous draft.
Zero-duration native pose clips open as one editable timeline frame.
**Prepare .anim import** opens Aurora's existing animation import inspector
with the same native bytes. Review its preview, destination, name and
permissions, then use the normal upload action. Preparing the inspector does
not start an upload or payment. The pose editor closes for this review and
its drafts stay available through Animations.

Encoding, preview binding, file dialogs and disk writes use background jobs.
Grid upload and playback of the resulting inventory animation are validated
by the human in Second Life.

## Offline validation

`AURORA_DEMO_IK_EDIT=clip` creates head rotation and pelvis position keys at
frames 0, 30 and 60, then displays frame 15 on the Bone page, the head
selected in the picker. `clip-frame` displays frame 45;
`clip-priority` shows the per-bone priorities on Tracks;
`clip-settings` displays loop/ease metadata on Clip; `clip-crop` trims frames 15–45;
`clip-import` prepares the ordinary native import inspector without uploading;
`clip-release` closes at frame 720 (capture 800 to check restoration).
