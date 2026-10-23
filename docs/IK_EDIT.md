# IK Edit and general bone contacts

Open **Build > IK Edit** or press **Ctrl+K** while your avatar is visible.
The shortcut is configurable in Preferences and restores a minimized panel;
pressing it again leaves the current edits open. In a focused script editor,
Ctrl+K retains its line-comment command. This is a local,
temporary authoring tool: it freezes the avatar's skeletal animation output,
draws its skeleton through the mesh, and lets you adjust the displayed pose.
Closing the window resumes animation playback and discards the temporary
pose. Contact authoring reads simulator objects and exports script source.
**Animations > Create an animation from this pose** opens the related
[native animation creator](ANIMATION_CREATION.md), with keyed posing and
`.anim` save/import preparation.
The `ik_edit` viewer command toggles the same tool. The shared Aurora tab
strip separates **Projects**, **Bindings**, **Virtual bones**, **Animations**,
and **Display**, in that order. **Options** and the two export buttons stay in
a shared bottom panel across the authoring tabs, outside their scrolling content.
General options (World/Local space, Symmetry, Reset pose and the script IK
mask) sit on the left. The four limb activations form a two-column block
on the right: left/right hands above left/right feet. Its import icon copies
active script contacts.
Bindings opens first; selecting a saved row selects its source.
**Link to a prim**, **Link to a bone** and the edited-prim shortcut belong to
the selected-bone card, immediately below its name. They append a binding to
the profile chosen in the selector immediately before these buttons. This
selector and the profile card below stay synchronized.
The IK settings section starts collapsed, containing avatar-frame transforms
and chain controls; bone reset and attachment distance remain directly
accessible. The profile selector, smoothing and paste controls use compact
rows so the first associations fit in the default window without scrolling.
Binding names share a full line with the target
pencil. Influence, orientation following, copy and deletion are grouped below,
with deletion at the right edge. Only the selected association exposes its
offset/contact fold. Search the list by source or target name; multiple words
can match both names together. Filtering never changes the pose or exported
data and clears when switching profiles or completing target designation.

The **Projects** tab contains the project selector and **Create a new IK project**
menu, available from the initial screen. The menu offers **Blank project**, **From an object or
attachment**, **Import a local file**, and **Import a notecard**. For a blank
project, confirm its proposed **New IK project** name, then choose **Create a new IK profile** and confirm
the proposed **New IK profile** name. Names are selected for immediate
replacement by typing. Cancel creates nothing. Empty projects contain no
implicit profile. **Projects** remains available from the empty editor;
**Bindings** permits first-profile creation, and the remaining authoring tabs
become available after confirmation. Existing saved projects remain available
through **Projects** and **Load an IK project**. Each import receives a fresh local identity; the currently open
project is saved before switching to the imported one.

**IK profiles** group bindings, smoothing and base animation layers below the
selected-bone inspector. The plus asks for a new profile name before creating
it; duplicate uses the adjacent icon. The pencil renames a profile and changes its script selector.
Each profile previews only its own bindings and animation stack. Duplication
copies binding calibrations and animation settings independently; shared
virtual controls retain their identities. Copy a binding with its row icon,
then paste it into the selected profile. Its referenced virtual hierarchy is
copied with fresh control IDs. Deleting a profile retains bindings shared by
other imported profiles and remaps their slot references.

The **Animations** tab has its own profile selector and edits that profile's
base animation stack. Drag the grip to reorder layers; the arrows remain available.
Native joint priorities apply, with the higher row winning ties. Each layer
keeps its enabled state and numeric/slider frame. Live-animation suppression
belongs to the stack. These layers are calibration references, not commands
to play animations on other viewers.
Priority and inventory-drop reminders are available on hover instead of
permanent instructional rows.

An **IK project** groups a complete set of profiles for one creation, for
example **Motorcycle** or **Glasses**. Its read-only name is centered above the tabs.
All management controls live inside **Projects**: use its selector or list to
load an account-local project, and its adjacent icons to create, save or reload.
File/notecard exports belong to each project row; imports and draft recovery are also here;
the shared footer contains only IK controls and the two runtime clipboard exports.
The **Projects** tab lists saved projects with left-aligned names, a highlighted
current selection and a name filter. Click a project's name to save the current
work, load that project and open **Bindings**. Each row has right-aligned
**Rename**, **Duplicate**, **Export file**, **Notecard** and **Delete** icons, in
that order. File export opens a local save dialog and preserves the complete
project, including current unsaved edits when exporting the open project.
These actions apply
to that row without changing the open project or its working pose.
Rename/duplication asks for a name inline. Notecard exports the
complete project, including all profiles, animations, frames and their order.
Deletion asks for confirmation and removes only that local project. Deleting
the open project clears its editor; other projects stay open. Existing inventory
notecards remain independent copies.
A new project
starts without any profile; duplication copies all existing profiles, bindings,
virtual controls and animation settings under a new project identity.
Project titles do not change the profile keywords used by object scripts.

**Target object**, inside **Projects**, associates the whole project with
a linkset. Choose the object by clicking it or any of its child prims, or use
the single object currently selected in Edit. Worn body attachments work too;
HUDs and avatars are excluded. Escape cancels designation without changing
the project. Selecting a replacement updates all profiles and virtual-control
parents together, retaining their offsets, rotations, weights and animation
settings. It does not require sitting on that object or any IK script.

Named prim targets resolve by exact, unique name within the chosen linkset
(for example Left and Right on another motorcycle). Seated avatars and their
attachments are excluded from that object's members. Missing/duplicate names
remain unresolved rather than following the previous object's prims; a warning
identifies this state. Late property replies resolve them automatically.
Scene picks capture the containing object root and the link name, preserving
the exact prim frame and all calibration. A root pick uses `_ROOT_`. Unknown
names finish loading before the binding can be exported. These references
survive project saves and script exports; external targets keep their own root.
Unknown property names are fetched in batches of at most four per 250 ms,
with bounded linkset traversal. Only references to the project's source object
follow its replacement; other target objects retain their root UUIDs. The
chosen object and its name are saved with the project and duplicated with it.
If it disappears, the project remains available for choosing a replacement.

Named projects are stored locally per account and autosaved in the background
after changes, at most once per second while open, with a final close flush.
Switching, creating or importing another project saves the current work before
replacing it. The save button writes immediately; reload reads the last stored project.
Unfinished unnamed work is preserved as its own local project before a switch.
A malformed/missing destination leaves the current editor intact. Names are
labels; UUID filenames prevent path characters in a title from selecting a
folder. The local catalogue contains up to 128 projects.

The **Project** row in the shared footer exchanges `.aurora-ik.json` files and
inventory notecards. The project includes all profiles, binding calibrations,
virtual controls, animation order/frames, suppression settings and selected
profile. **Restore local draft** can recover an older unnamed draft; the
adjacent cross keeps the current work instead. File exchange also provides
portable snapshots. The notecard save icon creates an **Aurora IK - <project>**
card in the inventory's Notecards folder. File and notecard loading are in the
new-project menu above the tabs. Both formats carry the same versioned project, bounded to
64 KiB. Inventory writes happen only on request.

Animation references use original inventory item IDs, so moving a folder or
item does not break them. The project also retains the asset ID and name.
Item references are fetched once when needed; missing or trashed animations
stay in their saved position with their frame and can be replaced using the
pencil. Animation assets are not embedded and their permissions are unchanged.
Project files and cards contain editor data; they are separate from the
compact scripted rig export below.

**Copy complete IK script** copies the configured companion LSL source.
**Copy IK data** copies a ready-to-paste LSL declaration, `string RIG = "…";`,
containing the rig JSON definition: bound bones,
targets, contact/position/rotation offsets, influence, orientation following,
activation and virtual controls with their parent hierarchy and motion.
Both buttons briefly show **Copied** after a successful action, without moving
the footer. A copied binding briefly shows a check in place of its copy icon.
It uses the same escaped declaration as the complete script; legacy
prim contacts are converted to that format too. Replace the custom script's
existing `RIG` declaration with this line. Its JSON has no session envelope.
A custom publisher supplies discovery, session/multipart
transport and runtime timing, following `rig-contacts.lsl`. Virtual controls
can be exported before assigning a bound bone. Empty or invalid exports are disabled.

The authoring editor works without any LSL script, seat activation message
or incoming IK definition. It starts with its own empty contact list.
In the shared **Options** footer, **Ignore script IK while editing** is on initially: it hides
received constraints locally while their scripts and target updates
continue normally. Turning it off shows them again; closing the editor
restores their current effects. Editor constraints remain active under this
mask. **Import active script bindings** explicitly copies current script
contacts into editable authoring data without replacing existing authored
bindings. No animation-stop, script-stop or script-activation message is sent.

**Create a new IK project > From an object or attachment** starts scene
designation. Click any prim of the source linkset, or use the single prim
already selected in Edit. Sitting is unnecessary. Escape cancels designation;
the pending-request cross cancels a transfer. The source must contain the
current IK companion. The Aurora bridge updates to version 1.33 to relay this
explicit targeted request.

The viewer requests a fresh full declaration using a dedicated nonce and zero
cache hint. All declared profiles, bindings and virtual controls create a new
project named after the object, with exact offsets, weights and source-relative
parents. Temporary runtime masks do not rewrite the declaration. Editor-only
animation references cannot be recovered from the compact object payload;
file/notecard project imports retain them. Source deletion, region changes,
closing the editor and a 45-second timeout retire the request. Partial, stale,
wrong-source or malformed replies leave the current project intact.

The temporary authoring session is isolated from active avatar constraints.
It accepts only the selected source, uses the existing four-per-second request
budget, retains at most an 8 KiB declaration within the normal global budgets,
and releases its data after import/cancellation/expiry. Automatic discovery
still ignores unused objects. Reading a declaration neither activates IK nor
changes an existing runtime session. No object inventory or script source is read.

## Author a contact

1. Start the animation you want to adjust (for the motorcycle, sit and enter
   the ready/riding phase), then open IK Edit.
2. In **Display**, show only the bone sets you need. Select a named
   joint by clicking its skeleton point or searching the filtered bone list,
   then open **Bindings**. The selected source bone is highlighted in a themed
   card at the top, with its two direct binding actions.
   Selecting a source or bone target from a saved association reveals its
   display group when needed, without activating an inactive contact.
   Translation arrows and plane handles appear by default. Hold **Shift**
   to hide them and drag a point freely in the camera plane. Hold **Ctrl**
   for rotation rings; Ctrl-dragging the point itself turns it about the
   camera axes. Ctrl takes precedence if both keys are held. There is no
   Gizmo switch. **World** uses
   scene axes; **Local** follows the selected bone's orientation. This space
   selector is in the shared **Options** footer and also applies to virtual bones.
   Bound rotation keeps the contact target fixed. Unbound, parented rotation
   pivots at the selected joint and carries its contact point with it. Numeric position and
   rotation fields remain in the explicitly labelled avatar frame.
   Hold Alt to use the normal camera controls.
   Click anywhere along a visible bone segment to select its origin joint:
   upper arm selects the shoulder, forearm selects the elbow. Hover highlights
   the segment and shows its name; selected segments are teal. Terminal joints
   also show their canonical tip. Point handles stay selectable and take
   priority near an articulation. Shaft clicks select only; use the point or
   gizmo to manipulate the pose. Virtual parent connectors select their node.
   The same shaft clicks work when choosing a bone target for a binding.
   The teal ring marks the selected bone's contact point.
   **Show handles** shows large effectors and labelled bend diamonds for
   activated limbs only. Hand/foot grips and their manipulation axes stay
   directly on the joint/contact. Shift-drag a grip to pose the limb, or select it
   and use the gizmo. Clicking a grip does not change calibration.
   Bone-target picking uses the real skeleton points.
   Ctrl temporarily selects rotation: hold it
   before grabbing a rotation ring, or Ctrl-drag the effector point itself.
   An already grabbed axis/ring keeps its operation until released.
   Elbow/knee diamonds stay at their actual bend targets, with larger hit
   areas and a visible connector. Shift-drag a diamond to choose the bend plane while the
   hand/foot target stays fixed. Selecting it shows translation arrows at
   the bend target. Ctrl exposes rotation rings centred on the actual
   elbow/knee joint; the rings orbit the bend target without changing the
   hand/foot orientation. The grabbed gizmo operation stays fixed until
   release. Numeric bend-target fields appear when selected.
   **Symmetry**, also in the shared **Options** footer, mirrors subsequent edits onto the canonical left/right
   counterpart across the avatar's XZ plane. Rotations and contact offsets
   use both bones' rest frames, rather than negating Euler angles. Existing
   opposite-side edits are replaced when a mirrored edit is committed;
   turning symmetry off leaves each side independent. Reset this bone resets
   its counterpart too while symmetry is on. Central bones are not paired.
   The shared **Options** footer has separate activations for the left/right hand and
   foot. Enabling one reveals its bone group and
   handles and expands its chain: hands can recruit the torso/chest/collar;
   feet can recruit the pelvis above the hip. A reachable target uses the limb
   alone; more distant targets can rotate the body without stretching segments
   or translating bones. Full-strength contacts sharing body joints are
   reconciled together within the solver's iteration limit. Partial influences
   retain their single local-transform blend. The selected source card's chain-root selector in **Bindings** can
   restrict the influence explicitly. Export the updated script to keep the
   selected roots in game. Imported and existing pins retain their explicit
   roots when disabled and re-enabled. For independent handlebar contacts,
   select `mShoulderLeft` for `mWristLeft` and `mShoulderRight` for
   `mWristRight`: the arms then leave the torso's animation unchanged.
   Broader roots remain available for deliberate body-assisted reach.
   Finger chains keep the animation's knuckle bend plane as their contact moves,
   including when a locked wrist limits their influence to the finger itself.
   This plane follows the hand's orientation instead of using an arm's outward
   elbow direction. Explicit bend targets still take precedence.
   **Global smoothing** is always visible at the top of **Bindings**. Its
   **Activation duration** and **Release duration** apply to every existing
   binding and new associations. Both default to 0.2 seconds; zero is immediate.
   Imported rigs keep their authored durations until these controls are edited;
   a notice identifies imports with different durations across bindings.
   The selected bound bone's card always shows its **Attachment distance**,
   the maximum distance in metres between the
     animated contact and its target before starting (0 means unlimited).
     Measure the animation without that binding's own correction. Other
     active contacts can bring it into range: a wrist pin can therefore
     enable a nearby finger contact, without making proximity self-latch.
   Distance defaults to zero and remains independent for each binding.
   The object script chooses when to request activation; the viewer adds no
   activation delay. It starts the entry fade as soon as the target is available
   and within the radius. Obsolete `delay` fields in installed data are discarded.
   Once active, the distance gate stays latched until release. These settings
   are exported in each binding's `activation` object (`distance`,
   `ease_in`, `ease_out`), including the global durations in each binding so
   existing companion scripts and compatible observers read the same values.
   They apply in game; pose editing stays immediate.
   At full influence, an oriented parent contact retains its frame while
   child contacts solve below it. For example, an index chain starting at
   a bound wrist can bend the index without rotating that wrist away from
   its own target. This applies to both authoring and live playback; a
   partially influenced or position-only parent keeps its remaining freedom.
   Parent protection starts with its entry fade, keeping that fade continuous
   instead of changing the child's chain when full influence is reached.
   During live playback, shared body assistance recovers toward the current
   animation as its reach permits, while maintaining the hand/foot contacts.
   An earlier correction does not permanently add another torso lean when
   the creation changes or restarts its animations.
   Disabling removes the effector/bend markers. These apply constraints; **Show handles**
   in **Display** only controls visibility.
   Disabling a limb removes its own correction without erasing calibration.
   It can still follow shared body rotations driven by other active contacts;
   disabling all contacts restores the frozen animation pose. The editor still pauses animations
   globally until it closes. A subsequent manual edit starts at the displayed
   joint, keeping its chosen parented/detached mode; it updates/reactivates that
   bone's association when one exists. An unbound edit changes the local pose
   without enabling a hand/foot pin. Enable the limb switch explicitly to hold
   its contact in place.
   Selecting the bone, refreshing targets
   or adjusting strength alone never reactivates a disabled contact.
   Re-enabling the limb switch restores IK with its body-assisted chain.
   Association rows own activation and influence;
   the contact editor's Pose tab does not repeat them. Native animation
   authoring keeps its own controls because it has no Bindings tab.
   Script contacts can be imported explicitly as separate editable bindings.
   **Display** offers independent torso/head, extended spine, face, arm, leg, finger,
   wing, tail, hind-leg and other sets. Click the selected group labels to
   hide/show them, or use **All**, **None**, or the **Human** preset (torso,
   arms and legs). The filters hide skeleton segments, joint/pole handles,
   selected highlights and gizmos, and narrow the bone list. They never
   change bindings, weights or enable states. The Human preset is active
   initially. The four extra Bento spine pivots are hidden in it; torso/chest
   lines connect through them to the visible human parents. Their transforms
   stay in the actual skeleton and animations. Enable **Extended spine** or
   **All** when those additional joints are needed.
3. Manual rotation of an unbound bone pivots that joint; its children follow,
   including children already edited in this session. It never solves or moves
   the ancestors. Unbound movements solve the chosen chain once and retain its
   local pose, rather than creating a permanent target. Explicit bindings and
   enabled limb handles continue to hold their targets.
   Leave **Detach** unchecked in **Bindings** to move through an ancestor chain
   without stretching it. Choose its root in the selected source card. Two-segment chains
   use an analytic solver; longer chains use bounded CCD (at most twelve
   joints and sixteen sweeps), with body-assisted canonical limbs as above.
   Checking **Detach** instead directly changes the bone's local transform, including
   translations; this works for the pelvis, facial joints and other bones
   for which preserving a limb chain is inappropriate. Descendants follow.
   All bones with a parent start with **Detach** unchecked, including the
   torso/chest, neck/head, fingers and face. Releasing or reselecting a bone
   never detaches it automatically. Existing saved constraints retain their
   explicit mode. The skeleton root has no parent: it remains freely movable,
   and its Detach checkbox is unavailable and unchecked.
4. In free manipulation, hold **Ctrl and drag the
   selected joint's point** to rotate it. Horizontal
   and vertical movement turn around the camera's up/right axes. Bound contact
   targets stay fixed; unbound children follow their rotated parent. Releasing Ctrl during the drag switches back to
   translation without a jump. Position-only IK keeps the selected joint's animated local articulation.
   Enable the rotation-arrow button (**Follow rotation**) on the binding row to match the target rotation as
   well as its position; its tooltip explains the position-only alternative. Editing
   the rotation fields enables that option. Click a value to type it, or
   drag the value horizontally. Ctrl-drag enables orientation automatically
   and updates these fields too. Rotation fields are XYZ degrees
   in the avatar's frame, composed in the LSL-compatible ZYX order.
   The selected source card in **Bindings** groups its link actions on one row,
   then **Avatar frame** (collapsed initially) and **IK settings**. Position in
   metres and rotation in degrees share aligned XYZ columns. **Bind to a prim**
   and **Bind to a bone** add an association, including another target for the
   same source joint. Each association row has its own **Offsets and contact**
   section, closed initially, with target-relative position/rotation (**Target
   frame**) and source contact calibration (**Bone frame**). Select a row's bone
   to edit that association with the gizmo or avatar-frame fields; the pencil
   replaces only that row's target. Weight, orientation and activation are
   independent per row. The chain root and Detach share a row; attachment
   distance and bone reset share the next. The import icon in the shared **Options**
   footer provides the explicit script-contact import action, with a tooltip.
   Options stay visible while scrolling or changing tabs and are disabled during
   target designation to preserve the locked source. IK Edit has no separate Pose tab;
   the native animation creator keeps its own posing page.
5. Each binding's contact point is measured in bone-local metres *before* joint scale.
   It defaults to the bone pivot. Adjust its numeric XYZ values for the worn
   mesh when another contact location is useful. Changing the contact point keeps the currently displayed bone
   in place. The ring is the point to align to the handle.
   Ankles also offer **Use sole point**, starting at `(0.08, 0, -0.05)`;
   calibrate it for the worn foot mesh and footwear. Calibration changes only
   that association. Simultaneous targets for one bone are blended by their
   authored weights and runtime fades; total strength is capped at 100%. One
   joint cannot occupy incompatible targets simultaneously. Target dependencies
   resolve first, and oriented contacts are converted to joint-origin targets
   using each binding's material point and the avatar's actual joint scale.
6. Click **Bind to a prim** or **Bind to a bone** below the selected source.
   Click the exact prim in the world or a bone on your own skeleton.
   The source stays selected while choosing the target. A successful click
   creates the association immediately, preserving the source's authored
   pose. Use the row’s **pencil** menu, then **Prim** or **Bone**, to
   replace its target;
   Escape leaves target picking and preserves the existing association. A self-reference,
   missing target or dependency cycle is rejected without changing it.
   Other-avatar target selection is not offered in this authoring interface.
   Virtual control points can also be clicked as bone targets.
   IK Edit and the object Edit tools may remain open together. If exactly
   one prim is selected in Edit, **Use edited prim: …** offers it directly;
   use **Edit linked** for a child prim. Scene designation leaves the object
   Edit selection unchanged. The scene picker remains available for a
   virtual control's prim parent in **Virtual bones**.
   A unique link name, such as `Left` or `Right`, resolves that link again
   after copying the creation. The picker retains that name automatically;
   the duplicate name field is removed. Each external target retains its own
   object root UUID and link name in the exported data.
   Object manipulator handles have precedence where the world handles overlap;
   window controls remain above both skeleton and object handles.
7. New associations follow both
   position and rotation. Moving/rotating the bound bone edits its metric
   target-relative offset; moving/rotating its target carries that bone.
   One target per bone replaces the previous association. The **Bindings**
   list selects, enables and deletes associations. A missing target releases
   the affected constraint until it becomes available again.
8. Each saved association row reads left to right: activation, source bone,
   target name, target pencil, **Target strength** slider/numeric percentage,
   and delete. Clicking the source selects its bone. Clicking a prim name opens
   object Edit and selects that exact linked part for moving/rotating; IK Edit
   stays open. Known prim names update from object properties, with a loading
   label while the name is unavailable, rather than displaying a UUID. This
   display name does not rewrite the script's link-name/UUID resolution.
   Set strength
   from 0 to 100%. At 0% no correction is applied;
   100% applies the full solution, subject to reach. Intermediate values
   blend solved local rotations/positions with the base pose, preserving
   IK segment lengths. This is an influence weight, not a linear percentage
   of world-space distance. It affects both position and orientation.
9. The footer has two export actions. **Copy complete IK script** copies the
   full companion with every saved association, virtual hierarchy, animation, offsets, contact
   points, chain roots, poles, orientation/mode choices, weights and enabled
   states. **Copy IK data** copies the equivalent `string RIG = "…";`
   declaration for direct use in a custom publisher. Disabled associations retain their calibration and authored
   weight in the script's initial disabled-bone list. Paste the whole result
   into the root prim's companion script. Individual fields have no separate
   clipboard buttons. Runtime activation policy belongs to the LSL companion
   or the creation's main script; it is not an editor setting. The supplied
   templates keep their `EXTERNAL_CONTROL` field for that choice. Editing and
   previewing do not depend on it.

**Reset this bone** in the selected source card in **Bindings** removes its
edit; **Reset pose** in the shared **Options** footer removes all edits. Native
animation creation keeps these controls and its chain/space choices in Pose
because it has no Bindings tab. Closing and
reopening starts afresh. Teleporting or logging out exits the mode. Other
avatars continue animating. An unreachable IK target is clamped without
stretching; residual errors above 5 mm are displayed. IK has no anatomical
joint limits. Canonical extended limbs use shared body assistance, while
short shoulder/hip roots isolate each limb. Incompatible contacts can still
leave residual error; inspect the resulting pose.

## Compose an animation pose

In **Animations**, add one or more inventory animations, or drag an animation
from Inventory onto the section. The picker loads the standard Animations
folder and searches already known animations, including inventory links.
For a custom folder, open it in Inventory or drag its animation directly.
Animation assets load through the ordinary cache and background asset streamer.
Unavailable assets contribute no pose and show their loading/error state.
The authoring stack is bounded to sixteen entries.

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
does not hide another layer's position track. Unkeyed bones retain the frozen
base pose. Layer frames and asset data stay unchanged when reordered.

**Exclude current avatar animations** starts from the shaped rest skeleton,
preserving mesh joint offsets and scale, instead of the frozen live pose.
This option affects the local authoring preview; it sends no animation-stop
messages and does not reset the AO or the simulator's playing animations.
Turning it off restores that frozen base. Closing IK Edit resumes the
existing animation pause clock. The selected animation samples are applied
before IK, so saved prim contacts still constrain the final pose.
These layers are temporary authoring tools; the companion publishes bone
contacts, while the creation's existing scripts remain responsible for
runtime animation selection and phase changes.

## Install the script on the motorcycle

Click **Copy complete IK script** and paste the entire result into one LSL
script in the root prim. It uses [rig-contacts.lsl](../scripts/lsl/rig-contacts.lsl)
for every configuration, including simple prim contacts. Keep the vehicle's
main script. The exported `RIG` JSON retains bound bones, target objects/links,
metric position/quaternion offsets, contact points, roots, bend planes,
orientation following, strength, activation and virtual controls.
Use unique link names for reusable targets: copying/rezzing a creation
changes its UUIDs. Missing or duplicate named links release their targets.
Every binding and virtual parent identifies its target independently:

```json
{"kind":"link","object":"source","link":"Left"}
{"kind":"link","object":"01234567-89ab-cdef-0123-456789abcdef","link":"Grip"}
{"kind":"link","object":"01234567-89ab-cdef-0123-456789abcdef","link":"_ROOT_"}
```

`object` is the target object's **root UUID**, or `"source"` for the publishing
object's current root. The latter survives copying/rezzing without rewriting
JSON in LSL. The exact, case-sensitive `_ROOT_` token always means the root;
it never searches for a child named `_ROOT_`. Other names must match exactly
one member of that object's linkset. Different objects may use the same name.
A profile can combine any of these per-binding targets; its slot list, offsets,
contact points, smoothing and activation are unchanged. No target grouping or
child prim UUIDs are needed in the declaration.

The viewer resolves names for occupied/worn declarations and the open editor.
It asks for missing properties at most four times in each rolling second. The
available queries leave on consecutive frames instead of waiting 250 ms each,
so small cold linksets resolve sooner with the same traffic budget. There are
at most three attempts per prim in each finite lookup, then it stays quiet. Known names
are reused. Every member name, including the root, uses an individual
ObjectSelect/ObjectDeselect read. A read immediately releases its temporary
server selection, never deselects a prim already selected in Edit and never
changes local object selection or transforms. The four-per-second budget counts
reads; an unselected prim read sends two small messages. Topology changes, or a new declaration with unresolved names, can
start one fresh lookup; unchanged profiles/state updates do not. Fresh replies
replace metadata without dropping unrelated verified contacts. Received name
changes update the map. Project saves and binding copies retain the same
root/link pairs, including while metadata is loading. Each viewer bounds this session cache to
64 objects and 1,024 prims (256 prims per object, names at most 256 UTF-8 bytes).
Editor targets, the local seat and local attachments take priority over observer
metadata when admitting objects and scheduling queries; existing entries stay
stable within each priority.
Missing, duplicate, deleted, unlinked or out-of-region targets release only their
dependent constraints. `_ROOT_` targets need no property query. Retiring declarations
or changing regions drops unused metadata. The companion transmits definitions
unchanged; it performs no link-name lookup or JSON target rewriting.

The script defaults to `EXTERNAL_CONTROL = TRUE`, for a
phase-controlled motorcycle main script. It starts
released, asks that main script for its state, and accepts:

```lsl
llMessageLinked(LINK_SET, -190018108, "on", NULL_KEY);
llMessageLinked(LINK_SET, -190018108, "off", NULL_KEY);
```

Use `FALSE` for a standalone creation that should activate whenever its
rider is seated on it or its owner wears it. Creators decide when contacts apply by sending these
messages; they define how each contact works through the exported definition.
Per-bone conditions use `"off mWristLeft"` / `"on mWristLeft"` (substitute
any configured bone). A global phase `"off"` overrides them; global `"on"`
retains those individual conditions. The publisher's disabled-bone list is
bounded by the configured definitions and resets when the script resets.
Every global off/on cycle creates a fresh activation epoch, even when the
viewer receives both messages between frames. Restarting discards the old
fade, release correction and body seed, matching a fresh seated activation.
It does not restart unrelated avatar animations. Repeated `on` messages while
already enabled preserve the current activation. Per-bone mask cycles also
reset their own envelopes without restarting other bones' timers.
Use a newly exported complete companion script for the activation-epoch header;
the viewer requires it. The existing `RIG` data can retain default parameters.

Definitions are sent in bounded multipart messages; there is no continuous
script pose stream. LSL compilation and behaviour on a real motorcycle
require human validation in Second Life.

### Named IK profiles

`RIG` is the single declaration of virtual controls, binding targets, offsets,
weights, contact geometry and entry/release parameters. Its optional `profiles`
array selects subsets of those shared bindings by a short keyword:

```json
"profiles": [
  {"name":"drive", "bones":["mWristLeft","mWristRight"]},
  {"name":"gas_only", "bones":["mWristRight"]}
]
```

Bindings can belong to several profiles; their data is declared only once.
Up to 16 profiles and 16 bindings are supported. Keywords contain 1–64 ASCII
letters, digits or underscores; profile members must be unique existing bones.
With profiles present, only the selected profile's bindings may run. Without
profiles, `on`/`off` controls the ordinary complete rig.

```lsl
llMessageLinked(LINK_THIS, -190018108, "profile drive", NULL_KEY);
llMessageLinked(LINK_THIS, -190018108, "profile gas_only", NULL_KEY);
llMessageLinked(LINK_THIS, -190018108, "off", NULL_KEY);
```

Send a command only when the gameplay/pose mode changes. Repeating the current
selection is idempotent. State changes arrive through the separate bridge IK
relay. There is no periodic discovery or lease-renewal request in a live session.
Reloading the same definition does not release contacts or restart entry smoothing.
A switch from `drive` to `gas_only` releases only the
left hand through its configured ease-out; the right hand keeps its existing
contact and fade clock. Switching back starts the left hand's own entry
conditions again. Each binding has a source-owned generation counter, so a
viewer can detect reactivation even if the intermediate release packet was
missed. A complete off/on cycle also retains the fresh global activation epoch.

Keep `EXTERNAL_CONTROL = TRUE` when the main script chooses modes. On `sync`,
reply with its current profile or `off`. With `FALSE`, an eligible source starts
with the first declared profile (or all bindings without profiles). Standing,
detaching, source deletion, teleport and lease expiry still release effects.
New viewers retrieve the declaration and the current selection together.

No animation name, UUID, inventory permission or animation-state observation
participates in this selection. The main script plays its normal animations
for every viewer and chooses the IK keyword separately. IK Edit posing stays
independent of these runtime commands; imported profiles survive calibration,
retargeting and both exports. Virtual controls retain their source clock while
switching profiles. To restart every active contact, explicitly send `off`
followed by the desired profile.

## Frames and protocol

For target prim position/orientation `P/Q`, displayed bone transform `B`
including its shaped scale, and bone-local contact point `c`:

```text
position = inverse(Q) * (B.transform_point(c) - P)
rotation = inverse(Q) * rotation(B)
```

Prim scale does not enter these metric offsets. Avatar joint scale does
enter the contact point. Rendering and solving use the same interpolated
linked-object transforms and shaped skeleton as the avatar renderer.

A seat wins conflicting bone definitions for its rider, followed by owned
worn roots in stable UUID order. Removing one source preserves the others.
The same runtime handles all avatars; the observer contract below explains
discovery, authorization, transport and automatic release.

## Virtual controls and runtime graphs

The **Virtual bones** tab lists each added virtual bone with its named parent
(real bone, prim or another virtual bone). Selecting a row opens its inspector.
They are manipulators, not new skinned joints or simulator prims. Parent a
control to an own real bone, prim or another virtual control. Numeric fields edit its parent-local
position and XYZ rotation in degrees in **Placement**; **Control animation**
opens the separate motion inspector. The point and the same world/local
gizmo edit its displayed transform. Translation handles appear by default;
Ctrl exposes rotation rings and Shift allows free point dragging. The
space choice is shared from Bindings, without duplicate tool buttons here. Selecting a
real bone returns manipulation to that bone. Deleting a branch removes its
descendant controls and associated bindings.

A control can be static, keyed or procedural. **Key at current time** records
the translation/rotation delta after its base offset. **Auto key** records
visual edits at the current time; with it off, a visual edit moves the base
of the entire path. Play/pause, rewind, time slider and numeric seconds
control the local preview. Key interpolation offers step, linear, smooth and
cubic Bézier easing, with draggable curve handles and numeric coordinates.
Procedural motion offers sine, triangle, sawtooth, pulse and ramp functions,
position/rotation amplitudes and phase. Both support once, loop and ping-pong.
Changing a keyed duration rescales its key times. Rotation keys interpolate
quaternions; procedural XYZ amplitudes use the same ZYX composition as LSL.

Exports containing new bindings/controls use
[scripts/lsl/rig-contacts.lsl](../scripts/lsl/rig-contacts.lsl). Set its
`EXTERNAL_CONTROL` field inside the script for standalone startup or the
motorcycle's existing `on`/`off` messages; per-bone messages remain supported. Only one
publisher is installed per root. Sources may be a seat or a worn
owned attachment. The runtime retains read-only remote-avatar references
for compatibility, although the authoring interface currently offers only
own-avatar bones. Each link target is scoped to its declared object root;
an external target does not need to publish its own IK declaration.

Protocol `auroraIK/5` sends bounded base64 fragments (up to 512 characters
of data per message), an authenticated nonce, revision, bundle ID, activation epoch, enabled
state, selected profile, per-binding activation generations, disabled-bone
bitmask and virtual-control clock. Full definitions commit
atomically after bounded, out-of-order reassembly; invalid/incomplete
transfers cannot replace a valid graph. Off messages
release immediately. Cached graph heartbeats do not resend keyframes.
Publishers pair adjacent 256-character fragments when the sampled header leaves
room in the existing 880-byte relay payload, and otherwise use one. The choice
is fixed for that snapshot; low free Mono memory also retains the original
small packets. Packet cadence and ingress limits stay unchanged.
This reduces declaration messages and timer waits without speeding up gestures
or their authored fades. Existing protocol-5 viewers accept the packed data.
An avatar's initial transfer finishes before another of its unknown sources is probed. Definitions
are limited to 16 controls, 16 bindings, 64 total keys and 8192 JSON bytes;
individual keyed controls allow up to 32 keys and durations up to 600 s.
The exporter disables invalid/oversized configurations.

The shared evaluator resolves own-bone and own-attachment targets from the
current working pose, orders dependent constraints before their followers,
and releases cycles plus their dependants. Unrelated constraints continue.
Other avatars pose independently in parallel; their final read-only frames
are refreshed before the owner's final solve. Runtime animation clocks are
anchored when the script is enabled. An elapsed phase plus a monotonic arrival
anchor supports scripts older than the computer’s uptime, and f64 phase
reduction keeps long-running cycles moving. No continuous joint-pose stream is sent.


## Compatible observers and lifecycle

If a scripted profile has no visible effect, trigger it and press **J8** in
Dev Toolbox (Ctrl+Alt+D), then Copy. This local report distinguishes relay
reception, decoder failures, complete definitions, accepted activations,
editor suppression and unresolved/duplicate prim names. It adds no discovery
or property queries and does not include authentication IDs or full rig data.

The complete export uses `rig-contacts.lsl` for simple contacts as well as
virtual rigs. The driver and every observer use the same `auroraIK/5`
contract. Earlier prototype scripts are unsupported; replace their complete
source with a new export.
Each compatible viewer receives definitions and solves them for the affected
avatar using that PC's current animation, shape and object transforms.
Unmodified viewers do not apply these corrections. The rider does not need
to run a compatible viewer for observers to see the scripted effects.

There is no simulator IK registry. The viewer installs **#Aurora Bridge IK**
as a third script in its existing bridge attachment, independently of movement
and read-only services. On an authenticated relay handshake after login, arrival,
region crossing or relay restart, the viewer asks it to discover regional sources
once. The script emits `auroraIKDiscover|<relay session UUID>` with `llRegionSay`
on channel `-190018107`. Only publishers with a seated rider or worn attachment
reply. An object also announces when its use or configuration changes.
Replies to a regional discovery are scheduled after a random delay of 0–0.5
seconds to spread the response burst. Existing timer ticks and simulator
scheduling may add latency. Timers run only while those replies or
definition fragments are queued; the relay itself has no timer.
Live sessions do not rescan avatars every second or renew leases every 25 seconds.
The ordinary object parentage still authorizes each subject and cleans up use.
At most four targeted requests per second, 256 sources and 128 observed avatars
are admitted, with at most four simultaneous multipart streams.
Incomplete requests have at most five retries; they do not restart
an endless discovery loop. Offline demo fixtures can deliver the same snapshots
directly without provisioning an attachment.
An off event received before the definition releases corrections but does not
count as a completed load. Finite retries continue until that definition is
complete. Replacing a bundle requires its own complete load; late state from
the old bundle cannot acknowledge the replacement.

Only currently occupied seats and actually worn attachments are candidates.
An unused object's availability announcement or unsolicited data is discarded,
without creating a session, storing a definition or adding it to chat history.
The companion announces `auroraIKAvailable|<bundle UUID>|<user UUID JSON array>`
regionally on the negative channel when usage begins; it never sends rig data
in that announcement. Listening bridges forward the event privately to their
viewers. The viewer requests data only after verifying a listed user's actual
seat/wearer parentage. Availability is bounded to sixteen users and 880 bytes.
It preloads the full declaration even while the object's IK phase is off.
An announcement can precede delivery of its source or seated avatar. A separate
16 KiB bounded identity-only declaration queue retains root, owner, bundle and
subject IDs, without loading any rig. Full object updates recheck those specific
declarations when parentage arrives or a source reappears; terse motion and idle
frames never launch a discovery scan. An empty user array clears the declaration
and applicable runtime source. Region changes discard the declaration queue.

There is no persistent rig catalog or disk cache. During live use, definitions
are kept only in working memory, including between IK off/on phases while the
same rider remains seated or the same attachment remains worn. Data is dropped
when that use ends. Incomplete transfers expire after 15 seconds without needing a
further packet. Completed editor-reload snapshots expire after 45 seconds.

Each definition is limited to **8192 UTF-8 bytes**. Combined serialized data,
including partial replacements and distinct editor snapshots, is limited to
**32 KiB per avatar** and **128 KiB across the current region**. A rejected
replacement preserves any valid current definition. Schema limits on controls,
bindings and keys apply independently. These byte budgets bound the payload;
decoded structures and bounded session metadata also use memory.
Incoming protocol traffic is limited to 64 messages/second globally (burst 128)
and 8 per source (burst 64). Availability has its own 64-per-second/burst-128
admission limit and shares the outgoing request budget. Repeated announcements
of the same bundle cannot retire an in-flight nonce or restart failed retries.
Excess, invalid and ignored traffic is discarded rather than saved as chat.

The viewer sends a negative-channel `ScriptDialogReply` to its worn bridge root
on channel `-190018106`, index 0. An authenticated relay session routes either
`auroraIKRelay|<session UUID>|discover` or:

```text
auroraIKRelay|<session UUID>|observe|<root UUID>|<affected avatar UUID>|<nonce UUID>|<cached bundle UUID or zero UUID>
```

The relay's owner-filtered listener verifies the session and forwards the request
to the publisher using `llRegionSayTo` on channel `-190018107`:

```text
auroraIKObserve|<root UUID>|<affected avatar UUID>|<nonce UUID>|<cached bundle UUID or zero UUID>
```

The publisher checks that the bridge's owner is an avatar in its region and that
the affected avatar is actually seated in its linkset or is its wearer.
The viewer verifies the same relation independently, using simulator objects,
and verifies that chat source/owner match the publisher root. An object
cannot designate an arbitrary standing avatar as its subject.
Full ObjectUpdate messages may omit OwnerID when there is no sound or particle
system. That absence is not an ownership mismatch: the viewer authenticates
the root through simulator chat source/owner and live seat/wearer parentage.
Any known conflicting owner is rejected, and a worn publisher's chat owner
must still be its wearer. This also permits discovery when the companion is
installed after the viewer has already probed the seated object.
`llRegionSayTo(bridge, -190018107, ...)` delivers full definitions only to the
requesting bridge. The relay forwards each packet using `llOwnerSay` with
`<auroraIKRelay><session>|<original sender>|<original owner>|<message></auroraIKRelay>`.
The viewer consumes only messages from its exact worn bridge root and current
relay session, then validates the original source with the ordinary source and
byte-budget checks. Raw envelopes never reach chat or logs. Adaptive fragment
packing reserves room for this envelope inside OwnerSay's 1024 bytes; large
headers retain the original 256-character chunks.
A bounded transfer queue retains independent viewer/avatar nonces and cached
bundles. Up to 128 pending requests are retained, subject to free script memory;
records are removed after completion or abandoned-request expiry without
invalidating viewer data.

Each `auroraIK/5 ` packet contains the affected `avatar` UUID and:
`nonce`, monotonically increasing `revision`, `bundle`, non-nil `epoch`, `enabled`, `mask`, selected `profile`, per-binding `cycles`
and elapsed control `time`. Full definitions additionally contain zero-based
`part`, `count` and base64 `data`; state-only packets omit those three fields.
Definition bounds are described above. Nonces belong to a
single viewer, affected avatar, root and region session. Out-of-order parts
commit atomically; old revisions/nonces cannot resurrect released state.
After a complete definition, no data is sent while state stays unchanged.
Phase changes broadcast one small `auroraIKState/1 ` JSON event to listening
bridges, including explicit off messages. It carries the same state fields as
a snapshot header except `avatar` and `nonce`: original root identity and actual
parentage identify existing sessions. Old revisions cannot undo newer state.
The publisher generates a new `epoch` UUID on each global disabled-to-enabled
transition, preserving the bundle when the definition is unchanged. The viewer
uses it to discard old fade/body state even if no disabled pose was rendered.
The companion also sends bounded per-binding generation counters (`cycles`),
including releases/reactivations that an observer missed. A named selection
requires one counter per binding; values are nonnegative signed-32-bit integers.
Multipart fragments must agree on profile and counters. Unknown keywords, invalid
counter shapes and mixed transfers cannot replace a valid state. Reloads preserve
these lifetimes. Only this version-5 contract is
accepted; install the current complete companion when updating the viewer.

Standing, detaching, deleting the source or losing the avatar invalidates
the source from live parentage, without waiting for a script off message.
Missing targets release their dependent constraints. Teleporting or changing
the main region discards negotiated state and requires a new nonce; logout
or disconnection resets the world and all contact sessions. Detaching the relay
also releases corrections; live sessions do not expire simply because no state
changed. Consequently, stopping a publisher script without removing its object,
or losing its final state event, requires explicit resynchronization or a later
state event; this event-only design does not silently add periodic health checks.
Corrections remain separate from
the underlying animation pose and release with each binding's configured fade.
The own-avatar editor and its script-IK mask do not freeze remote avatars.

## Offline validation

`AURORA_DEMO_IK_EDIT=1|move|release` reuses the synthetic motorcycle and
opens the editor at frame 420. `move` edits the left palm position and
orientation. `release` makes that edit, then closes at frame 720 without
standing the rider, so the original motorcycle contacts must return.
Capture frame 620 for authoring and frame 800 for restoration.
`library` shows a named motorcycle project and its complete profile set;
`library-name` opens the inline duplicate-project name field.
`library-target`, `library-target-pick` and `library-target-missing` show the
project's object, click-designation hint and unavailable-object warning.
`library-menu`, `library-object`, `library-object-import`,
`library-object-waiting` and `library-notecard` show the creation menu,
scene designation, completed object import, finite waiting state and inventory
notecard picker.
`library-empty`, `library-new`, `library-profile` and `library-profile-name`
show the initial create action, proposed project name, empty named project
and proposed first-profile name respectively. No implicit entry appears.
`links-add`, `links-bone-pick`, `links-bone`, `links-prim` and `links-visible`
show the guided target choices, the locked-source designation step, saved
own-bone/prim associations and the visibility tab. `links-menu` opens the
row's Aurora pencil menu; `links-pose` shows selected-source details in Bindings.
`links-multiple` binds one wrist independently to Left and Right with 40/60%
weights and expands the second association's calibration.
`links-offsets` selects the right wrist, binds Right and opens the offsets/contact
inspector, scrolled to show the three coordinate frames together.
Options and exports occupy the same fixed footer in these modes, `graph-keys`,
`animations` and `links-visible`. `shortcut` opens the panel through the Ctrl+K
command binding at frame 420 and repeats it at 520 without closing the editor.
Headless pointer events
exercise the actual buttons and skeleton handles; target-replacement tests
also cover pose preservation, cancellation and rejected dependency cycles.
`rig-bindings` associates all four limbs with the two demo targets at 100%;
`rig-weight` uses 50%; `rig-disabled` creates them then disables all four at
frame 520. Compare the latter with the underlying paused riding animation.
`rig-elbow` and `rig-knee` select a bend control and show its joint-centred
rotation rings. Bend diamonds stay at their real targets; hand/foot grips
and gizmos stay at the contacts. `rig-body` advances both hand targets beyond
the local arm reach while retaining both foot contacts, showing torso assistance.
`rig-groups` keeps only the arm set visible: leg/face/finger/appendage
segments, handles and poles must disappear while all four goals stay active.
`animations` shows two independent samples at frames 15 and 45;
`animations-reordered` reverses their equal-priority order, `animations-frame`
changes the first frame and disables the second, `animations-neutral`
disables both samples and excludes live inputs, and `animations-release`
closes at frame 720 (capture 800). Inspect the authoring tabs, adjacent frame
fields, visible sample differences and persistent export footer.
These are local fixtures, not real LSL compilation.

Pure regressions cover direct transforms across every named rig joint with
nonuniform scales and a pelvis offset, analytic arms/legs, longer finger
chains, unreachable/invalid targets, parent-before-child edits, snapshot
freeze/release, runtime fade/release, protocol validation, target-space
round trips, exported quaternions, editor reset, scaled-UI pointer rays and
palm-handle dragging among overlapping bones. Native captures check
the visible skeleton and floater layout. Human validation must additionally
check real pointer selection/dragging, camera gestures, clipboard exports,
the worn mesh, the actual shape and LSL compilation on the motorcycle.

`graph`, `graph-keys`, `graph-remote` and `graph-cycle` inspect parented
virtual controls, procedural/keyed motion and Bézier UI, remote bone targets,
and a deliberately cyclic own-wrist parent. The cyclic constraint is released
and its warning shown while unrelated script contacts remain available.

Named runtime targets stay inside their individually declared linksets. The
viewer resolves a root and a unique link name, independently of the publisher's
own linkset. Missing or duplicate names release dependent constraints without
discarding other bindings. Removing a prim from its declared object releases
its target immediately; the publisher need not send a new bundle.
