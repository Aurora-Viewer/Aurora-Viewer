# Local photo poser

Local trial for [issue #727](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/727),
based on the merged IK workbench (PR #893) and the supplied Poser
handoff and mockup. Issue #727 remains open while grid validation is pending.
The human validated the local demo on 2026-10-23 and authorized main integration on 2026-10-24. Two-client grid validation remains required.

The poser edits joint rotations and scales directly and moves joints through
their articulated ancestor chains. It reuses the shaped Bento
skeleton and transform tools from the IK editor; it does not evaluate IK
projects or create bone-to-prim/bone bindings. Its selection, overlays and
pose history are independent of the binding editor and virtual controls.

## Posing

Open **Build > Poser**, or add Poser to the toolbar. In the photo mode's
viewfinder, which hides the rest of the interface, the dock's wand button
opens it over the shot (#1389): its joints take their clicks before the
focus, and a worn HUD keeps its own. The window follows
the layout validated on 2026-10-24: the subject strip (subject, posing
status, "Visible to you only" with the full notice as tooltip, Start / Stop),
the tabs, the joints column on the left and the selected joint on the right,
then the file footer. The subject button opens a menu with yourself and your
owned animesh roots, then every nearby avatar with its consent action (see
Consent); Privacy › "Who can ask to pose my avatar" is its last entry. Other subjects must be within 50 m, in the same region and unmuted.
Several subjects can retain independent poses at once. Eligibility is
checked every frame; losing it removes the corresponding local pose.

### Compact window

The poser opens in a small window by default (an Aurora addition: the
Legacy Viewer's poser has one window). The title bar's ⤢ button switches
to the full window described above, and ⤡ back; the choice is remembered,
and the other window opens with the same top right corner. Both windows
share the subject, the pose, the undo history and the settings.

- **Head**: the subject's picture, name and state (a teal dot that breathes
  while posing). The picture opens the subject menu. Every other avatar
  posed, or who accepted, has a small picture at the right (a "+n" past
  four): a click poses that one.
- **Groups**: Body, Face, Hands, Other and Physics in one segmented strip.
- **Transform, depth of field and side**: Rotation / Position / Scale, the
  depth of field switch, then Left / Right / Mirror.
- **Selected joint**: a disclosure row with its name (a violet dot when it
  is edited) and its values (X · Y · Z). Opened, it shows Fine-tune:
  Handles Show / Hide, Values Total / Change, and an X, Y and Z slider,
  the full window's.
- **Last row**, three groups of one kind each: picking (markers Always or
  On hover, Ctrl + click body parts, Ctrl + click avatars), going back
  (undo, redo, reset) and files (Save, Load, Upload). The reset greys out
  when it would change nothing; its tip names the joint; with Shift it
  resets the whole pose. Save is a small form: the name, BVH, pelvis,
  priority, Save to the library, and Save as… (the file chosen in the
  system dialog). Load lists the library, with the load mode in a submenu
  and Open file.
- **Start / Stop**, alone across the window under the last row.
- **Options** (the title bar's gear): only the settings the window does not
  show itself: stop posing when the window closes (and the question before
  losing the pose, shown while it is on), natural rotation, and the
  question before replacing a pose. The full window's settings hold them
  all.

Posing in the compact window goes through the markers and gizmos in the
world, and the Fine-tune sliders; the joint list and the trackball are in
the full window.

**Other parts with Ctrl** (Ctrl + click body parts on): holding Ctrl over a
part that another group governs (a hand while posing the Body) tints it in
the accent, with "Ctrl + click: Hands" by the cursor, and Ctrl + click
selects its nearest joint and opens its group. Without Ctrl nothing shows
and the click stays the world's (in the photo mode, a plain click on the
face still sets the focus, and a Ctrl + click on a part stays the
poser's instead of passing to the world). The body is one part, a hand reaches its
wrist, the face is an oval that follows the head, and the wings and the
tail are offered only when a worn rigged mesh uses them.

The subject menu (both windows) uses the People list's rows: the picture,
the name over the account name, and the distance at the right in Near
you's colours. A nearby avatar's square button reads Ask, Asked (a click
cancels), Accepted (a click poses them), Refused or No answer.

**Markers on hover** (Settings › Handling, and the compact window's eye /
hand button): the joints and shafts fade in while the pointer is over the
avatar being posed, and out when it leaves; the selected joint's gizmo
stays, and a drag keeps them.

**Clicks in the world**, in both windows (Aurora's own: the Legacy Viewer
changes joint from its list only): a click that no joint, shaft or gizmo
takes unselects the joint, and the pelvis is not picked again by itself (a
click a worn HUD takes leaves it selected); Ctrl + click on another avatar
posed, or who accepted, poses that one at once (with Ctrl held, their
skeleton shows under the pointer). A plain click on them stays the
world's: a photo's focus, a HUD drawn over them.

**The Other tab** lists only the bones a worn rigged mesh uses or the pose
moved (Aurora's own: the Legacy Viewer lists them all). The bones inside the
body (the spine, the groin, the skull) are left to the full window's list,
even used, posed there with the sliders and the trackball, with no markers
nor handles in the world: the spine's are collapsed pairs whose markers sit
upside down over the torso and whose moves tear the body apart, and the
body's own bones bend it. Unused, the wings, the tail and the hind limbs
rest unseen.

In the compact window's Other tab, a mesh that uses only the bones inside
the body says so and points to the full window.

**Ctrl and the handles:** where Ctrl shows another part or avatar under the
pointer, the selected joint's handles let the pointer through, so the Ctrl +
click reaches it (a rotation sphere over the body); elsewhere they keep Ctrl
(its temporary rotation, a handle's click in the photo mode), and a drag
already begun keeps them.

Reset on the selected joint, with Mirror chosen, also resets its
counterpart, as the Legacy Viewer's mirror does (`FSPoserAnimator::resetJoint`).

The pose captures the displayed shaped skeleton and overrides its final
animation palette. Body, Face, Hands, Other bones and Physics expose the
actual Bento joints and collision volumes. Whole-avatar movement is the
first Body row. Each joint pair has one row: the **Side** selector above the
list (Left / Right / Mirror) chooses which one is edited, and Mirror is the
symmetry (the left joint is edited and its counterpart follows). A joint
picked in the world shows its side in the selector. The Hands tab has the
five presets, applied to the chosen side (both in Mirror, in one undo step),
and a finger grid: finger × base, middle and tip. Edited joints have violet dots, including their
in-world markers.
Only the active tab's joints and shafts are visible and selectable. Changing
tabs selects a joint in that group and replaces the previous group's gizmo.

Use the Rotation / Position / Scale inspector, rotation trackball, axis
sliders or in-world movement/rotation/scale gizmos. The trackball's axes and
drag directions follow the current camera, including orbit and roll.
Horizontal and vertical trackball drags rotate the joint with the pointer.
Rotations use the joint's parent
frame in degrees; positions use avatar axes in metres; scales are factors.
Delta mode is
relative to the captured initial pose. Children follow their parent with
the normal skeleton scale rules. Parent offsets are retained during positional
edits: moving a wrist bends its shoulder/elbow chain, without detaching or
stretching the hand away from the forearm. A target beyond the chain's reach
is clamped. Only the unparented skeleton root translates freely. Explicit
scale edits still change proportions through the normal skeleton rules.
A world lock attempts to hold a joint's position and orientation when its
ancestors or subject move, yielding at reach limits to preserve attachments.
Scale has local X/Y/Z cube handles and a central uniform-scale cube. Drag an
axis cube outward to enlarge that channel, or drag the central cube right/up
to enlarge all three channels. Scale values stay bounded and keep their
sign: some shape sliders mirror a face or pectoral joint on one axis (Head
Shape at 100 does it to the forehead), and the poser keeps that axis.
Dragging a joint marker directly rotates or moves that joint, according to
the selected transform mode. This changes the pose without assigning a target.
Loaded pose/animation translations also retain the captured shaped offsets;
non-root displacement is retargeted through joint rotations.

The joint's header holds its own actions: the world pin (lit while locked),
Mirror (copies the joint to its counterpart) and Reset joint. The **Whole
pose** group holds Flip, Left → right, Right → left, T-pose and Reset all.
Mirror on the Side selector mirrors ongoing edits. The hand presets
Relaxed, Fist, Open, Point and Pinch follow the Side selector. Undo/redo is independent per subject and retains 64 checkpoints;
a continuous drag creates one checkpoint. The title bar's curved arrows undo
and redo, as do Ctrl+Z and Ctrl+Y (or Ctrl+Shift+Z), unless a text field has
the keyboard or the build tools' window is in front of the poser's (their
Undo then). Gestures bound to these keys wait while the poser is open. Reset joint / Reset all restore
the captured initial channels. T-pose restores the rig's neutral rotations.

Stop releases a subject's pose while retaining its edits for Start. Closing
stops posing by default; disable that setting to keep poses with the window
closed. Switching to IK editing, teleporting or logging out releases the
poser. Poses affect only this viewer, including poses of other residents.

## Library and files

Save reports the saved file’s full path. Load > Pose library opens the saved
poses directly; its Open folder button opens their directory in the file manager.
File pickers start in that same directory.

Save writes a named `*.aurora-pose.json` under the current account's local
`poses/` directory. It retains rotation, relative translation, scale and
world-lock flags, without subject IDs, avatar names or world coordinates.
The book in the title bar shows the library in the joints column's place
(the window keeps its size): back arrow, refresh, open folder, search, the
entries with their date; a double click loads everything. Save's menu (its
chevron) holds the optional BVH output next to the pose file, the pelvis
unlock and the priority. Load's menu holds the four modes, applied to the
selected library entry while the library is shown or to a file chosen
otherwise, and "Open a file".

Load accepts Aurora pose documents, native `.anim` files and `.bvh` files,
up to 4 MiB. Animation imports sample their first frame. The four load modes
are All, Rotations only, Unedited joints and Selective rotations. Selective
loads preserve joints already edited in the current pose. Loading finishes
on the subject originally chosen, even if the picker changes during I/O.
World locks are reanchored to the destination subject.

Files and uploads contain what you authored, as in the Legacy Viewer
(`FSPoserAnimator::getJointExportRotation`, `FSFloaterPoser::savePoseToXml`):
the pose captured when posing starts is what the changes apply to, and stays
out of them. **T-pose** clears it, so a pose built from there is saved
complete; a hand preset or a loaded file does the same for its joints. Reset
brings the captured pose back, and undo restores this state with the pose.
A pose document stores complete joints as they are and the others as changes,
which apply to the destination's own pose (the Legacy Viewer's "Save Diff").
Files saved before this are read as complete. BVH and uploads write the same
changes on the rest skeleton: an untouched capture exports a T-pose.

BVH export writes the actual skeleton hierarchy in inches and Y-up, with
a reference frame and two identical posed frames. The pelvis-unlock setting
controls root rotation and translation. Native animations and BVH have no
scale tracks; BVH also omits collision volumes. The Aurora pose document
retains those channels. **Upload as animation…** reuses the existing native
encoder and opens the normal import preview. Upload and the simulator price
confirmation still require the human's action; saving never uploads.

## Consent

The subject menu lists every nearby avatar without probing their viewer or
bridge. Ask needs the worn bridge (the request travels through its query
service): until that service answers the button is disabled and the menu says
why: the bridge is starting, it was taken off, it failed, or it is worn but
its query script stays silent for 10 seconds. The last two offer **Repair
Aurora bridge**, the Movelock window's Repair. Ask sends one explicit request, with Cancel while
pending. The target sees the requester, distance, the local-only photo use,
the session lifetime, Refuse, Accept and a permanent-request-block checkbox.
Accepted targets keep a reminder with Withdraw even when Poser is closed.

Privacy > Who can ask to pose my avatar defaults to **Friends**; Everyone
and Nobody are also available. Friendship permits a request, never posing
without acceptance. Muted or blocked requesters do not get a modal. No
answer after 60 seconds expires the request; refusal and timeout impose a
10-minute cooldown. Responses require the matching request ID and peer.
Withdrawal, disappearance, region changes, teleport and logout invalidate
the relevant access. The requester immediately drops an invalid pose and
receives a notice.

The bridge is version **1.34**, which triggers the existing reprovisioning
flow. The new service lives in `query.lsl`, separately from the movement
controller. HTTPS commands use the existing validated query endpoint and
token. Attachment-to-attachment traffic uses `llRegionSayTo`, request IDs
and action/target fields only. The receiving script authenticates the sender
with `llGetOwnerKey` and `OBJECT_ATTACHED_POINT`; the viewer additionally
checks the worn bridge root, owner and region before consuming the private
owner-say tag. There are no IMs, software advertisements or capability probes.

## Settings and validation

The gear of the title bar opens the settings as a panel over the joint,
grouped Window (stop on close, confirm a close that loses the pose, DoF
while posing), Handling (markers, natural
alignment, trackball sensitivity) and Files (overwrite confirmation, BVH,
pelvis unlock, priority), with the note on the formats' limits.

The eye next to the gear hides or shows the joint markers and gizmos in one
click, the same setting as Handling's switch: an open eye while they are
shown, a struck-through one once hidden. It follows the Legacy Viewer's
visual manipulators button (`FSFloaterPoser::onToggleVisualManipulators`);
Aurora keeps the choice at the next opening.

With stop on close, closing the window (its cross, Ctrl+W, the photo
viewfinder's wand) loses the edited joints of every subject. When some are
edited, a dialog asks first: **Minimize** keeps the pose in the window's bar,
**Close anyway** closes, Escape or a click beside it cancels; "Don't ask
again" turns the setting off. The Legacy Viewer closes at once
(`FSFloaterPoser::onClose`); Aurora asks so that no pose is lost by mistake.

Defaults were checked against the local Legacy Viewer source: stop on close,
joint markers and natural alignment on; DoF in edit, additional BVH, pelvis
unlock and overwrite confirmation off; trackball sensitivity 0.5. DoF while
posing permits the normal configured DoF effect, without enabling it globally.
Settings and privacy are saved through Aurora's existing settings store.

Main integration validation on 2026-10-24: the script parser, formatting and Clippy
pass; the complete workspace has 4,860 passing tests, zero failures and 29
ignored tests. The suite used one test thread on the shared PC. Twenty-three poser/consent
regressions cover actual pointer actions for both hands, visible scroll
clips, scaled text and window stability, pose/BVH/native files, eligibility,
history and authenticated consent lifetime. Rotation and articulated movement drags create one undo
step; idle controls preserve exact values, and stale IK bindings/virtual
controls cannot affect the poser. Camera orbit/roll is checked against
projected axes; pointer tests cover axis/uniform scale drags and tab-exclusive
marker sets. Seventeen native poser scenarios and a
separate IK bindings regression were inspected, with zero runtime errors.
This does not validate real two-client LSL delivery.

Offline demos use `AURORA_DEMO=1` and `AURORA_DEMO_POSER=` with `self`,
`connected`, `camera`, `scale`, `hands`, `face`, `misc`, `physics`, `library`, `settings`, `animesh`,
`consent-accept`, `consent-refuse`, `consent-timeout`, `consent-withdraw`,
`consent-request` or `consent-reminder`. Capture frame 620. The synthetic
request/answer events exercise the real consent state machine; they are
not a substitute for two-client LSL transport validation.

Human checks: edit each tab and channel, use presets and undo, save/reload
each load mode, enable BVH and inspect its import, confirm overwrite, prepare
an animation import, switch subjects, and test both close behaviours. On a
grid, use two accounts running this build to verify bridge reprovisioning,
request/accept/refuse/withdraw, mute/privacy, timeout, region departure and
logout. Also validate owned and unowned ground/worn animesh and mesh/shape
overrides. The agent never logs into a grid or performs an upload.
