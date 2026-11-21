# IK Edit

Open **Build > IK Edit** or press **Ctrl+K** while your avatar is visible.
The shortcut is configurable in Preferences and restores a minimized panel;
pressing it again leaves the current edits open. In a focused script editor,
Ctrl+K keeps its line-comment command. The `ik_edit` viewer command toggles
the same tool.

IK Edit is a local, temporary posing tool. It freezes your avatar's skeletal
animation output, draws its skeleton through the mesh and lets you adjust the
displayed pose. Nothing is sent to the region or to other viewers: other
avatars keep animating, and closing the window resumes animation playback and
discards the temporary pose. Teleporting or logging out leaves the tool too.
**Animations > Create an animation from this pose** opens the
[native animation creator](ANIMATION_CREATION.md), with keyed posing and
`.anim` save and import preparation. For photo poses, use the
[Poser](POSER.md).

The shared Aurora tab strip separates **Pose**, **Animations** and
**Display**. The **Options** footer stays under every tab, outside the
scrolling content: World/Local space, Symmetry, Reset pose and the four limb
switches.

## Pose a bone

1. In **Display**, show only the bone sets you need. Select a joint by
   clicking its skeleton point, a bone segment or a row of the filtered bone
   list. Clicking along a visible segment selects its origin joint: the upper
   arm selects the shoulder, the forearm selects the elbow. Hover highlights
   the segment and shows its name; selected segments are teal. Terminal
   joints also show their canonical tip. Point handles take priority near an
   articulation. Hold Alt to use the normal camera controls.
2. Translation arrows and plane handles appear by default. Hold **Shift** to
   hide them and drag a point freely in the camera plane. Hold **Ctrl** for
   rotation rings; Ctrl-dragging the point itself turns it about the camera's
   up and right axes. Ctrl takes precedence if both keys are held, and a
   grabbed axis or ring keeps its operation until release. Releasing Ctrl
   during a drag switches back to translation without a jump.
3. **World** uses the scene axes; **Local** follows the selected bone's
   orientation.
4. **Pose** shows the selected bone's card: **Reset this bone**, its
   avatar-frame position and rotation (metres and XYZ degrees, composed in the
   LSL-compatible ZYX order) and its IK settings. Click a value to type it, or
   drag it horizontally.
5. Manual rotation pivots the selected joint; its children follow, including
   children already edited in this session. It never moves the ancestors.
   Leave **Detach** unchecked to move a joint through its ancestor chain
   without stretching it, and choose the chain's root in the same card.
   Two-segment chains use an analytic solver; longer chains use bounded CCD
   (at most twelve joints and sixteen sweeps). Checking **Detach** instead
   changes the bone's local transform directly, translations included: use it
   for the pelvis, facial joints and other bones for which a limb chain does
   not fit. All bones with a parent start with **Detach** unchecked. The
   skeleton root has no parent: it stays freely movable and its Detach
   checkbox is unavailable.
6. **Symmetry** mirrors the next edits onto the canonical left/right
   counterpart across the avatar's XZ plane. Rotations use both bones' rest
   frames rather than negated Euler angles. Turning symmetry off leaves each
   side independent; **Reset this bone** resets the counterpart too while
   symmetry is on. Central bones are not paired.

An unreachable IK target is clamped without stretching; residual errors above
5 mm are displayed. IK has no anatomical joint limits: inspect the resulting
pose. **Reset pose** removes every edit; closing and reopening the window
starts afresh.

## Limbs

The **Options** footer has separate switches for the left and right hands and
feet. Enabling one shows its bone group and handles and holds its hand or
foot where it is while you pose the rest of the body. Hands can recruit the
torso, chest and collar; feet can recruit the pelvis above the hip. A
reachable target uses the limb alone; a more distant one can rotate the body
without stretching segments. Disabling a limb removes its own correction.

**Show handles** in **Display** shows large effectors and labelled bend
diamonds for the enabled limbs only. Shift-drag a grip to pose the limb, or
select it and use the gizmo. Elbow and knee diamonds stay at their bend
targets, with larger hit areas and a visible connector: Shift-drag one to
choose the bend plane while the hand or foot stays fixed. Ctrl shows rotation
rings centred on the elbow or knee, which orbit the bend target without
changing the hand or foot orientation. Numeric bend-target fields appear when
a diamond is selected. Finger chains keep the animation's knuckle bend plane,
which follows the hand's orientation.

## Display

**Display** offers independent torso/head, extended spine, face, arm, leg,
finger, wing, tail, hind-leg and other sets. Click the group labels to hide
or show them, or use **All**, **None** or the **Human** preset (torso, arms
and legs). The filters hide skeleton segments, joint and pole handles,
selected highlights and gizmos, and narrow the bone list; they never change
the pose. The Human preset is active initially. The four extra Bento spine
pivots are hidden in it; torso and chest lines connect through them to the
visible human parents. Enable **Extended spine** or **All** when those
joints are needed.

## Compose an animation pose

In **Animations**, add one or more inventory animations, or drag an animation
from Inventory onto the section. The picker loads the standard Animations
folder and searches already known animations, including inventory links.
For a custom folder, open it in Inventory or drag its animation directly.
Animation assets load through the ordinary cache and background asset streamer.
Unavailable assets contribute no pose and show their loading or error state.
The stack is bounded to sixteen entries.

Each row has enable, move-up/down and remove controls, plus an independent
frame slider and adjacent numeric field. Typing a frame and dragging its
slider address the same sample; values are clamped to the clip's last frame.
Native animation files store continuous key times without their original
frame rate. The editor explicitly uses **30 fps**, beginning at frame zero;
the final inspection frame clamps to the asset duration. These are exact
samples, without looping back at the endpoint or applying playback easing.

Native per-joint priority takes precedence. For equal priorities, the
**higher row wins**; moving a row up therefore increases its precedence.
Rotation and position remain separate channels, so a rotation-only layer
does not hide another layer's position track. Unkeyed bones keep the frozen
base pose. Layer frames and asset data stay unchanged when reordered.

**Exclude current avatar animations** starts from the shaped rest skeleton,
preserving mesh joint offsets and scale, instead of the frozen live pose.
This option affects the local preview only; it sends no animation-stop
messages and does not reset the AO or the simulator's playing animations.
Turning it off restores that frozen base. The selected animation samples are
applied before IK, so your bone edits and limb switches still shape the final
pose. These layers are temporary: closing IK Edit drops them.

## Offline validation

`AURORA_DEMO_IK_EDIT` opens IK Edit on the demo avatar and drives it through
its modes (gizmo, limbs, symmetry, display groups, animation layers, native
animation creation); [TESTING.md](TESTING.md) lists them and what to capture.
