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

Open **Build > Poser**, or add Poser to the toolbar. The window follows
the layout validated on 2026-10-24: the subject strip (subject, posing
status, "Visible to you only" with the full notice as tooltip, Start / Stop),
the tabs, the joints column on the left and the selected joint on the right,
then the file footer. The subject button opens a menu with yourself and your
owned animesh roots, then every nearby avatar with its consent action (see
Consent); Privacy › "Who can ask to pose my avatar" is its last entry. Other subjects must be within 50 m, in the same region and unmuted.
Several subjects can retain independent poses at once. Eligibility is
checked every frame; losing it removes the corresponding local pose.

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
to enlarge all three channels. Scale values stay positive and bounded.
Dragging a joint marker directly rotates or moves that joint, according to
the selected transform mode. This changes the pose without assigning a target.
Loaded pose/animation translations also retain the captured shaped offsets;
non-root displacement is retargeted through joint rotations.

The joint's header holds its own actions: the world pin (lit while locked),
Mirror (copies the joint to its counterpart) and Reset joint. The **Whole
pose** group holds Flip, Left → right, Right → left, T-pose and Reset all.
Mirror on the Side selector mirrors ongoing edits. The hand presets
Relaxed, Fist, Open, Point and Pinch follow the Side selector. Undo/redo is independent per subject and retains 64 checkpoints;
a continuous drag creates one checkpoint. Reset joint / Reset all restore
the captured initial channels. T-pose restores the rig's neutral rotations.

Stop releases a subject's pose while retaining its edits for Start. Closing
stops posing by default; disable that setting to keep poses with the window
closed. Switching to IK editing, teleporting or logging out releases the
poser. Poses affect only this viewer, including poses of other residents.

## Library and files

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
grouped Window (stop on close, DoF while posing), Handling (markers, natural
alignment, trackball sensitivity) and Files (overwrite confirmation, BVH,
pelvis unlock, priority), with the note on the formats' limits.

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
