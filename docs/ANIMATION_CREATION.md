# Create native animations

Open **Build > IK Edit > Animations**, then **Create an animation from this
pose**. The creation mode uses the same skeleton, move/rotation tools,
world/local gizmo, Ctrl rotation, symmetry and hand/foot IK handles.
Its Pose, Animations and Visible bones tabs keep posing, timing and display
controls separate. **Return to IK contacts** restores the existing contact
definitions and their enable states.

## Record keys

The initial frame records the displayed pose, including the current IK
result. Choose a frame with the slider or adjacent numeric field, pose the
avatar, then record it:

- **Key this bone** records the selected joint. When its IK chain is being
  edited, the affected ancestors are recorded too; symmetry includes the
  edited counterpart.
- **Key changes** records the joints affected by the staged pose edits.
- **Key full pose** records the real skeletal joints in the displayed pose.

Recording is applied after the pose has been evaluated. The temporary pose
correction is removed when its new sampled keys are ready, so influence
weights are baked once. Other unrecorded pose edits remain staged. Seeking
or playing clears these temporary corrections and displays the keyed clip.
Previous/next controls and nearby numbered keys navigate the selected bone.
**Remove this bone's key** removes its rotation/position keys at that frame.

Rotations are native local quaternions. Pelvis translations are relative to
the avatar root. Other joint positions are included when a direct edit
changes them; ordinary rotation keys retain the avatar's joint positions.
The authoring grid is **30 fps**, from frame 0 to 1800 (60 seconds). Native
files contain continuous key times rather than an original frame rate.
Position channels must stay within the native format's -5 to +5 metre range
per axis. An invalid capture reports an error without writing a partial key.

## Set priorities and playback

**Bones / priority** enables each track and sets its priority independently.
`-1` inherits the global animation priority. Disabling a track excludes it
from preview/export while keeping its keys in the draft. Turning off the final
track releases all keyed preview channels; exporting still requires an active
track with keys.

**Playback settings** contains:

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
**Capture current IK pose** to add the displayed result to the current draft.
This bakes the real skeletal pose; the LSL companion supplies live target
following during play.

Creating another animation retains the previous draft in the draft selector.
Up to sixteen drafts remain available during the viewer session, including
after closing/reopening IK Edit. Save `.anim` files to keep exported clips
before leaving the viewer. Outside trimmed keys and disabled tracks remain
in the session draft; an exported file contains the selected active clip.

## Save or import

**Save .anim** opens a native file dialog and writes the validated file.
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
frames 0, 30 and 60, then displays frame 15. `clip-frame` displays frame 45;
`clip-priority` shows the global/inherited and per-bone priorities;
`clip-settings` displays loop/ease metadata; `clip-crop` trims frames 15–45;
`clip-import` prepares the ordinary native import inspector without uploading;
`clip-release` closes at frame 720 (capture 800 to check restoration).
