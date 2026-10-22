# RLVa implementation status

The `rlva` task implements the Legacy Viewer's RLVa 2.4.2 command families,
targeting RLV protocol 3.4.3, referenced by issue #687. Human grid validation
is pending. Preferences > Protections > Enable RLVa is
off by default, including settings written before the option existed.
Disabling it releases all session restrictions and discards queued commands.
The preference persists; object locks never do. The native next-login flag
persists separately by avatar UUID after tploc or a locked sitting session,
forcing the last position while RLVa remains enabled. It initially defaults
to the last position, as in the reference. Disabling clears that flag too.
Its ordered, coalesced writes run off the frame thread and flush at shutdown;
offline demos never read or write this catalogue.

Reference: `RlvCommand`, `RlvHandler`, `RlvActions`, `RlvInventory`,
`RlvAttachmentLocks` and `RlvWearableLocks` in `indra/newview/rlv*.cpp` of the
Legacy Viewer (originally LGPL 2.1, Kitty Barnett). The upstream
[command documentation](https://docs.catznip.com/rlva/allinone/) is useful for
script authors; the command lists below describe this branch specifically.

## Current command families

- OwnerSay parsing, comma-separated commands, n/y and add/rem, clear with a
  filter, per-object idempotent restrictions, numeric reply channels,
  strict communication exceptions and periodic issuer garbage collection.
  Secure exceptions require each secure issuer's agreement; permissive=n
  extends agreement to all issuers. Behaviours without a strict form keep
  permissive exceptions, as in the native exception checker.
  Protocol replies use a dedicated one-packet send path, including empty
  replies and UTF-8 truncation at 1023 bytes, without user chat splitting.
- Chat: sendchat, emote, recvchat/from, recvemote/from, chatwhisper/normal/shout,
  redirchat, rediremote, sendchannel and sendchannel_except. Redirection
  deduplicates channels, obeys channel locks and never falls back to public
  chat when every destination is restricted.
  OOC messages and short slash commands follow native filtering. OOC is allowed
  by default; ellipses are shown by default. Turning ellipses off suppresses
  filtered incoming messages before history/bubbles and suppresses blocked
  outgoing text. Directed object chat and owned worn attachments are exempt
  from receive restrictions. Redirection uses a single bounded shout reply by
  default; optional splitting uses word boundaries and UTF-8-safe packets.
  Channel exceptions accept positive channels. sendchannel_except entries
  require its general restriction and use the same native secure-agreement
  check as positive exceptions.
  Redirected typing is hidden by default across chat, animation and the
  AgentUpdate state flag. The native option can enable all three; stop
  messages remain allowed and adding a restriction stops active typing.
- IM: sendim/to, recvim/from, startim/to; share and sendgesture.
  Distance exclusion ranges are intersected across issuers, including the
  unbounded/off-region case; specific recipient blocks still win.
  Blocked online resident IMs show a replacement line and return a busy reply
  with the incoming session UUID. Offline and Linden messages retain their
  text; blocked/staff messages never disclose a personal automatic response.
  Forbidden outgoing text is replaced both locally and on the wire. Group
  sends check the group UUID; conference sends check every other participant.
  Initial group invitations check their group UUID and conference invitations
  censor their sender. Existing multi-agent session traffic follows the
  reference's separate receive path. New resident/group/conference sessions
  and calls obey startim before changing existing state; open sessions remain
  available, including sessions still initializing.
  Context menus, Contacts, People and notification reopening check before
  creating a session, so a rejected target cannot acquire that exemption.
- Online private IM queries: exact @version, @stopim, @list and @except,
  filtered by mute, spam and friends-only policy before recvim. Listing
  restrictions or exceptions requires a native permission question by default;
  sharing once, remembering consent and declining are distinct actions.
  The remembered choice is a user preference. stopim closes an existing
  startim-restricted conversation and its call even when optional queries are
  disabled. Protocol replies bypass user IM locks, with bounded UTF-8 packets,
  busy/normal session fields and the offline-friend flag. Global disable drops
  pending replies and questions; IMs never become object command sources.
- Experience-origin controllers use the native temporary-attachment option
  (on), minimum experience rating (Moderate) and the reference's default
  blocked-experience list. Classification waits for cached details and AttachmentResources;
  a refused controller loses existing rules and cannot issue new ones.
  The check covers child prims and deferred actions, keeps clear/removal
  available, and retires a block on confirmed detach or global disable.
  An unresolved attachment name expires after five minutes.
  Independently of experience origin, the temporary-source gate uses the
  source packet's attachment state and AttachItemID marker. With the option
  off, these OwnerSay messages are ordinary chat: no command, query or clear
  is interpreted. This does not retroactively remove accepted restrictions.
- Movement: fly, jump, unsit, sit, alwaysrun and temprun;
  forced fly, setrot, sit, sitground and unsit.
  A forced ground sit leaves an object seat, waits for the confirmed stand,
  then rechecks the delayed action; a stand-return teleport takes precedence.
  A nil sit UUID aliases ground sit. Native ground sitting obeys sit, while
  forced object sitting still obeys every sit/unsit lock and only bypasses
  distance. Sit/stand notifications use the accepted actor and actual parent;
  getsitid returns the root linkset. Initial state and failed requests do not
  manufacture transitions. Disabling clears pending actors and actions.
- Teleports: tplm, tploc, tplocal and sittp, with named-region or global tpto.
  Optional radian headings survive name resolution; global coordinates retain
  double precision until their region origin is removed. Named lookups pass
  offsets, including negative offsets, through global-coordinate normalization.
  Current large regions retain their local coordinates; remote destinations
  use 256-metre tiles. Without a heading the wire look field is local position
  plus X; explicit headings stay unit vectors. Named lookups return
  a request ticket through the common action guard before teleporting, so new
  restrictions apply and superseded replies are ignored. Maturity deferrals
  also retain the issuer. Disabling cancels deferred scripted teleports.
  Native teleport entry points check restrictions before starting progress
  UI, moving a history cursor or simulating an offline transition. Chat home
  shortcuts, maps, favourites, landmarks and invitation acceptance use this
  path; network sends retain their final policy check.
  Refusals after a named lookup or a maturity response use the normal
  teleport-failure path, cancelling pending progress and history state.
  tplure/tprequest exceptions and accepttp/accepttprequest auto-acceptance
  retain maturity checks and recheck consent when an invitation is answered.
  Forced acceptance overrides teleport/status rejection; do-not-disturb
  responses are still sent on acceptance or an RLVa refusal. Blocked offers
  and requests use busy replies, without manufacturing a user decline.
  Invitation text is masked under recvim or showloc as appropriate.
  standtp records the pre-sit location and returns only after a successful
  forced sit followed by a stand.
- Actions: buy, pay, rez; linkset touchall/world/attach/attachself/attachother,
  touchhud/this, interact and fartouch.
  Vendor payments obey buy; resident payments obey pay. Fartouch includes
  the exact distance boundary; sitting/teleport distances remain exclusive.
  A worn removal-locked HUD cannot be hidden through the HUD switch and
  prevents wireframe rendering. Add-only locks do not impose that condition.
  Transparency highlighting also obeys edit and showloc.
- Inventory: detach, addattach/remattach, addoutfit/remoutfit,
  sharedwear/unwear, unsharedwear/unwear. COF changes are checked as deltas;
  native detach and attach messages are checked too. Scripted removal honors
  the case-sensitive nostrip marker on originals, containing folders and
  folded parent folders. A link's own label does not create that marker.
  Node/subtree attachthis/detachthis locks and same-issuer exceptions follow
  shared links and folded folders; inventory mutation checks prevent moves,
  copies, renames or deletion from invalidating folder locks.
- Forced wear: attach/over/all/allover, detach/all/me, remattach, remoutfit,
  through the native COF planner. Shared-folder queries: getinv, getinvworn,
  findfolder/s, getpath/new; attachment and outfit presence/name queries.
  addoutfit/this/all/over and overorreplace aliases share the same handlers.
  Folded point folders choose the attachment point. Sequential outfit
  transactions retain their issuer until attachment sync finishes; later
  queries wait. Missing shared originals are fetched with bounded retries.
  Forced wear and getinvworn share a collector, including gestures and
  single-level folder links. Linked outfits, links back into the operation's
  subtree and second-level links are excluded. Point folders fold only one
  object while attaching; gesture and clothing handling follow native folding.
  Attachment-name blocks are balanced, follow originals before links, and
  respect modify/no-modify precedence. Legacy folder names, unnamed shared
  wear, hidden locked presence replies and add/replace folder prefixes are
  user preferences with the reference's defaults. The default add prefix is
  `+`; an empty replace prefix disables that override. Presence queries can
  exempt the requesting issuer without allowing a forced removal through its
  own lock. Body parts remain visible. getinv excludes folded folders and
  findfolder chooses the deepest matching path in native traversal order.
  getpath returns the first original's shared path; getpathnew retains all
  paths, COF order and duplicates. They fold one parent and exclude originals
  outside #RLV; folder-lock link traversal remains independent. Relative force
  wear from a controller whose object has not arrived waits up to thirty
  seconds for attachment, then rechecks locks. Other issuers continue while it
  waits. Issuer cleanup and global disable cancel those callbacks.
  Folded folders contribute their own items without recursively collecting
  nested folders. Linked hidden folders are excluded; dot folder names are
  literal inventory children. Worn queries retain slash-containing names.
  Accepted folder gifts prefixed `#RLV/~` can file into a shared path (on by
  default while RLVa is enabled). Scripted deliveries match their transaction;
  paths contain at most four components including `#RLV`. Root/intermediate
  creation, movement and renaming wait for confirmed metadata and only the
  exact pending filing operation bypasses folder locks. Scripted accept,
  decline and completed shared-path notifications follow native behavior.
  Disabling cancels filing and restores ordinary filing for unanswered gifts.
  With unnamed shared wear off, automatic naming observes new confirmed
  arrivals: modifiable objects receive a lowercase point suffix, while
  no-modify objects use a created or existing folded point folder. It waits
  for metadata confirmation, bounds retries, ignores the login baseline and
  does not repeatedly rename an already processed attachment. Disabling
  cancels this workflow while late inventory replies still update the model.
  The attachment watchdog preserves locked COF links during replacement,
  adds before resolving an unknown point and removes only unlocked previous
  attachments. Lost locked inventory items are restored at their exact point
  after the simulator saves their asset (15-second fallback, 30-second
  retries). Temporary attachments use simulator-local ObjectDetach and never
  attempt an inventory restore. Disabling clears all pending recovery.
- Camera: setcam exclusive ownership, avatar/origin min/max distances,
  eyeoffset/scale, focusoffset, fov min/max, mouselook and unlock; forced
  offsets, fov, focus and camera mode, with distance/FOV queries.
  camdistmin/max, camzoommin/max, camunlock and touchfar compatibility names
  preserve independent removal; adjustheight and getheightoffset use the
  native hover-height path.
  Inactive camera limits reply with an empty string; active numeric replies
  use three decimals. UUID focus fits the object's scale by default, targets
  an avatar's posed head and retains object tracking; position focus defaults
  to zero distance. Offset restrictions also lock forced preset writes and
  native preference/shortcut changes. Reset and default-FOV forces use the
  reference's defaults; releasing the last FOV limit restores the user's value.
  Active FOV commands use the reference's physical bounds adjusted for window
  aspect; enabling RLVa without camera commands preserves the native view.
- Vision: per-object setsphere blend, fixed/variable blur, chromatic
  aberration and pixelation; avatar/camera origins, distance/value extension
  and tweened local modifiers. At most six sphere effects run in order on
  the GPU. Variable Gaussian kernels have a maximum radius of 256 pixels to prevent
  unbounded shader workloads from external commands.
  setoverlay supports image, opacity, tint, tween, linkset priority and
  alpha-mask touch filtering. GPU composition follows world labels, precedes
  HUD attachments and also applies to photos without the interface; opaque
  overlay texels suppress object hits while terrain navigation remains usable.
  setcam_textures/camtextures temporarily replace world diffuse/PBR base
  textures and particle textures, leaving avatars and attachments intact;
  getcam_textures reads the active override. showself/showselfhead preserve
  HUD visibility. setcam_avdist/camavdist silhouettes use the reference's
  dark colour and take precedence over render exceptions.
- Display: showinv, showminimap, showworldmap, shownametags and
  showhovertext/all/hud/world.
- Privacy: showloc, shownames (including strict UUID exceptions) and
  shownearby. Display names, rich chat, notifications, hover text, maps and
  radar use privacy projections while canonical identities and transcript
  filenames stay intact. Hidden-name avatar menus expose the reference's
  limited actions. Application links and place details filter labels and
  disallow copying a hidden location. New landmarks, picks and classifieds
  cannot reveal the current location; existing entries remain editable.
  Teleport offers retain only eligible recipients (permissive tplure
  exceptions or online friends granted map rights). Hiding the location
  removes its current persisted arrival and masks the navigation label;
  releasing restores it without changing destination coordinates.
- Queries: version/new/num, getcommand, getgroup, getsitid, getstatus/all.
  Command discovery includes strict restriction names and supported legacy
  environment components. `getoutfit` uses the historical protocol bit order;
  attachment name queries and forced removals use native point groups.
  Attachment-name replies preserve the names and casing of the avatar library.
  Forced inventory operations obey every issuer's locks, including their own;
  detachme exempts its own item from nostrip, while attachment locks still apply.
  Deliberate correction to the reference's old numeric HUD mapping: the HUD
  group uses each point's actual HUD flag. This includes current group-9 HUDs
  and avoids treating the group-8 Bento body points as HUDs.
- Groups: forced UUID/name activation and role-name prefixes, exact-match
  precedence, native asynchronous role fetching, 60-second throttling and
  the reference's one early change for the lock owner. Group/title writes
  recheck setgroup; simulator-triggered group changes restore the last
  permitted membership without flooding activation messages.
- Environment: exclusive setenv ownership, scalar/vector/texture get/set
  commands. The optional environment-lock preference defaults to on;
  disabling it rejects setenv add/remove commands while retaining force
  properties and discovery, matching the native NoSetEnv setting. Existing
  locks can still be cleared or released by global disable. Supported properties
  include legacy colour components and intensity, sun/moon angles,
  asset UUIDs and named library sky/day presets.
  Positive query-angle domains, exact axis cases and solar-glow component
  resets follow the native conversion helpers. Temporary overrides restore
  the user's local environment when released or disabled. Native environment
  changes respect the lock.
- Editing: edit, editobj, editattach and editworld at selection, local
  transforms and network writes, including material and media capabilities;
  viewtransparent and viewwireframe. viewnote/viewscript/viewtexture also
  hide already-open documents without discarding drafts.
- Script questions: acceptpermission handles only the reference's eligible
  control/attachment permissions; caution permissions and experiences keep
  their dialogs. addattach/tploc filter the corresponding permission bits.
  Attachment permission checks every actual point, including point-specific
  addattach/detach locks. Automatic attachment permission uses the simulator's
  ownership flag and requires a rezzed object; missing inventory metadata
  never turns a worn attachment into a rezzed object.
  notify reports restriction commands, including rejected/duplicate commands;
  clothing/bodypart notifications follow complete confirmed COF changes,
  after the initial loaded baseline, with all removals before additions.
  An inventory-generation guard avoids rebuilding the snapshot each frame.
  allowidle uses the reference's thirty-minute automatic-away interval.
- Debug extensions: session-local AvatarSex pseudo-value, aspect ratio and
  existing renderer/environment capability reads, respecting setdebug
  ownership. RenderResolutionDivisor reduces real 3D target dimensions,
  including depth and post effects, then upscales before full-resolution
  world labels, overlays, HUDs and UI. Window-space picks map to the reduced
  depth surface. `default` or disabling RLVa releases the override. Arbitrary
  settings are not exposed.
  AvatarSex preserves the exact accepted boolean text in its reply; `default`
  is not a pseudo boolean. Disabling clears that session-only value.

Preferences also expose the reference's default-on experimental command set
and unlocked-only attachment replacement. Disabling the experimental set
filters discovery and new commands; removal/clear stays available for existing
locks. This applies immediately instead of requiring the reference's restart.
Strict replacement rejects a target point containing locked attachments,
including when the real point becomes known only after attachment. The default
retains locked predecessors while replacing unlocked ones.

Preferences > Protections > RLVa tools opens the native inspector and console.
Restrictions and inventory locks show their controller, support filtering and
copying, and retain exception rows. Modifiers show camera values and live
sphere/overlay samples. The console executes through the ordinary World command
path with the avatar as its separate issuer. Channel zero answers stay local,
including delayed queries; objects cannot use that channel. Closing the tools
releases only the console's locks, modifiers and queued work. Command history
is bounded and never persisted. Optional received-command logging is off by
default and can omit duplicate/unset feedback; queued commands are labelled
as queued rather than claiming that a simulator has completed them.

`getcommand` advertises the handlers actually present. `version` retains the
legacy RestrainedLife prefix; `versionnew` uses RestrainedLove. `versionnum`
returns `3040300`, or `2092800` for a matching worn creator/name compatibility
entry. `versionnum:impl` returns `2040213`; an unknown numeric-version option
gets an empty reply. Creator UUIDs and full, case-insensitive name regexes
use semicolon-separated `creator:` / `name:` entries. The list defaults empty.
Regexes compile once per edit; invalid expressions are ignored and evaluation
is bounded. The MIT-licensed [fancy-regex](https://github.com/fancy-regex/fancy-regex)
dependency retains lookaround/backreference use cases of native Perl expressions.

## Human grid validation required

The maintainer requested integration on 2026-10-22 with RLVa disabled by
default and live validation retained in #687. This is approval to merge the
implementation, not evidence that every live comparison has passed. The
separate tester kit provides 221 cases, eight LSL probes, read-only query
and catalogue sweeps, textures and blank results. Keep unavailable cases and
documented differences explicit until functional parity is certified.

Before live scripts, inspect the offline viewer in both languages:

1. Open Preferences > Protections. RLVa starts off; subordinate options are
   disabled. Enable it, open RLVa tools and select Console.
2. Submit `@version=0,fly=n,getstatusall=0`. The local reply shows RLV 3.4.3,
   Fly is blocked, and the restriction appears with the console issuer.
3. Close the tools. Fly is released. Reopen them and repeat with an external
   demo controller active: its restrictions must remain after closing.
4. Disable RLVa with active restrictions. Camera, visuals, movement, chat and
   inventory return to ordinary behaviour, the active list clears, and the
   next-login override is cancelled. Re-enabling restores no old object locks.

For live validation the human logs in, using a disposable controller and a
test outfit. Compare the same commands and preferences in the reference and
Aurora, retaining issuer UUIDs and response channels when reporting a mismatch.
Test both successful replies and malformed/empty options, release/clear from
one of two issuers, then global disable while a response is pending.

- Live-script validation of location and identity restrictions across the
  implemented UI paths, including delayed name resolutions, exceptions and
  opening an already-visible window while a restriction changes.
- Environment/debug replies, wearable/attachment/sit
  notifications and permission entry points. Ground-sit and clothing handlers require live
  validation with delayed simulator responses and intervening restrictions.
- Resident/group/conference IM restrictions, online/offline and Linden
  receive cases, secure/normal issuers, temporary controllers without an
  experience, and automatic teleport acceptance with status rejection.
- Linked/folded-folder behavior and delayed callbacks; shared inventory gifts, experience-origin controller approval and
  attachment recovery across crossings.
- Media/touch interactions and surface-distance checks; edit,
  inventory and document entry points with live scripts.
- Live visual comparison of sphere/overlay effects, forced textures,
  silhouettes and camera focus, including native camera controls.
- Real controller handshakes, compatibility entries, source disappearance/
  reattachment across crossings, and relogin from a locked session. Verify
  disabling releases every restriction and cancels the next-login override.

Keep these checks open in TASKS.md and #687 after integration. No grid
connection or credentials are needed for offline tests.

## Offline validation

Final integration on 2026-10-22, rebased onto current main through #827:
`scripts/check.ps1` passes with 4,442 workspace tests, zero failures and 27
ignored, including 121 RLVa tests. Scripts, formatting and Clippy are clean;
the explicit Vulkan pixel test also passes. The tester kit's four command-source
hashes still match its original baseline after this rebase. Existing native
captures identify their pre-rebase build; disabled/released fixtures were
replayed without interrupting the human's normal-mode test viewer.

`cargo test --workspace rlva` exercises parsing, enable/disable, multiple
issuers, strict exceptions, redirection, UTF-8 filtering, reply channels,
camera ownership, garbage collection, shared paths and clothing locks.
Run `scripts/check.ps1` for the required complete workspace checks.

Validation on 2026-10-22, including merged PRs #805, #807, #809, #810, #812 and #814: `scripts/check.ps1` passes with 4,415 workspace
tests passed, zero failures and 27 ignored; scripts, formatting and Clippy
are clean. This includes 121 RLVa logic/integration/shader tests, covering
environment conversions, command notifications, permission filtering,
teleport consent, edit/material/media guards, folder scopes/exceptions,
inventory moves, folded point folders, ordered outfit transactions, camera
compatibility, IM ranges, stand returns, visual-effect lifecycle/tweens,
alpha masks, shader validation, display-only privacy, surface distances,
local teleports across region edges, encoded empty/UTF-8 replies, attachment
recovery timing, saved-asset ordering, locked/nostrip replacement, temporary
attachments, changing attachment points, allowlisted debug writes, real target
dimensions and pick-coordinate conversion, registered privacy commands and
strict exceptions, location history, landmark/pick/classified writes, mixed
teleport-offer recipients, restricted native name menus, longest Unicode/URL
matches, active/inactive camera replies, focus defaults, preset mutations,
command discovery, historical outfit order, attachment groups and own locks,
group/name/role selection, asynchronous loading and precise restoration,
confirmed seat/ground transitions, root-seat queries, sit/stand actors and
legality, delayed-action cancellation and ordered clothing/bodypart notices,
aspect-dependent FOV bounds, OwnerSay case folding and malformed clear,
attachment-name casing, double-precision destinations and teleport headings,
deferred actors and one-use resolution tickets. An offline packet test proves
that map resolution emits the ticket without sending a teleport itself.
Inventory conformance also covers balanced names, link/permission precedence,
gesture COF planning, folder-link boundaries, folded content rules, deepest
folder search, old/partial preference defaults, prefix modes, hidden locked
presence replies and the exact nostrip scope. Native French enabled and
English disabled captures show the inventory options and their disabled state.
IM-query tests cover consent, deduplicated questions, exception formatting,
remembered permission, friends/block/receive filters, session closure,
disable cancellation, Unicode splitting and encoded busy/normal IM fields.
Native EN/FR captures show the real consent card while IM locks are active.
Rename tests cover baseline/new arrivals, preserving later manual names,
no-modify folder creation and confirmed moves, bounded failures, reattachment
and global-disable cancellation. The native French `rename` capture and log
confirm `Modifiable bracelet (chest)` and the `.(skull)` destination.
Communication and permission conformance additionally covers native busy
session UUIDs, local/wire substitution, offline/staff/typing exemptions,
group/conference recipients, existing sessions, startim before native session
creation, status/automatic teleport acceptance, actual attachment points and
simulator ownership flags. Secure/normal issuer combinations, channel
exclusion activation, vendor/resident payment guards, exact touch boundaries
and locked HUD visibility are checked too. Native FR/EN `communication`
captures and logs show the replacement messages and a blocked new session;
the EN `temporary` and FR `hud-lock` captures/logs confirm their source gate,
HUD visibility and filled rendering.
The additional Vulkan test
`cargo test --workspace rlva_gpu --locked -- --ignored` checks five sphere
modes, chained masks, reduced-scene upscaling and overlay alpha holes from
real GPU pixels without egui. It passes, including the extended upscale
and overlay-composition case.
These checks cover the implemented scenarios; they do not certify live-grid
parity with real controllers and simulator timing.

Initial native preference validation (`4e22e952`): the dev viewer builds
successfully. Inspected offline captures
`.scratch/rlva-disabled-fr.png`, `rlva-enabled-fr.png` and
`rlva-released-en.png` (1440 × 1000, frame 850): the switch is off with no
locks, on with the synthetic restrictions, then off with an empty list.
Labels, hints and session controls fit the existing Protections layout.

Native vision validation on 2026-10-22: inspected all five sphere modes,
the FR overlay with alpha holes, forced textures and release captures.
With the newer fixed avatar framing, `resolution` (FR) visibly renders
coarse 12-pixel blocks while the UI remains sharp; `resolution-released`
(EN) restores full detail. These are offline presentation checks, not live
script or complete-parity acceptance.

Native conformance captures on 2026-10-22: inspected `privacy` (FR),
`privacy-except` and `privacy-released` (EN), and `nearby` (FR). Names and
current-location labels hide, the chosen speaker remains visible through
the strict exception, disabling restores the names/place, and Nearby is
empty. Inspected `camera-focus` (EN), `camera-prefs` (FR), `head` (FR),
`self` (EN) and `silhouette` (FR): camera focus targets the avatar's head,
restricted preference fields are disabled, own head/body visibility follows
the commands and the distant other avatar becomes a dark silhouette.
These are offline native checks, without human grid acceptance.
The `sitting` FR capture and its native demo log confirm one object sit,
one stand and then `ground_sit=true, object_seated=false`. Fixture actions
are delivered once per rendered frame, even when native updates repeat.

`AURORA_DEMO_RLVA=enabled|disabled|released` opens the native preference page
and delivers synthetic object commands through `World::apply`. Capture after
frame 800. The three modes show active locks, ignored commands and locks
released by the global switch. These fixtures do not establish grid parity.
