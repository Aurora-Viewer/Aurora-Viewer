# RLVa implementation status

The `rlva` task is an experimental port in progress, referenced by issue #687.
It is **not full RLVa support**. Preferences > Protections > Enable RLVa is
off by default, including settings written before the option existed.
Disabling it releases all session restrictions and discards queued commands.
The preference persists; object locks never do.

Reference: `RlvCommand`, `RlvHandler`, `RlvActions`, `RlvInventory`,
`RlvAttachmentLocks` and `RlvWearableLocks` in `indra/newview/rlv*.cpp` of the
Legacy Viewer (originally LGPL 2.1, Kitty Barnett). The upstream
[command documentation](https://docs.catznip.com/rlva/allinone/) is useful for
script authors; the command lists below describe this branch specifically.

## Current command families

- OwnerSay parsing, comma-separated commands, n/y and add/rem, clear with a
  filter, per-object idempotent restrictions, numeric reply channels,
  strict communication exceptions and periodic issuer garbage collection.
  Protocol replies use a dedicated one-packet send path, including empty
  replies and UTF-8 truncation at 1023 bytes, without user chat splitting.
- Chat: sendchat, emote, recvchat/from, recvemote/from, chatwhisper/normal/shout,
  redirchat, rediremote, sendchannel and sendchannel_except. Redirection
  deduplicates channels, obeys channel locks and never falls back to public
  chat when every destination is restricted.
- IM: sendim/to, recvim/from, startim/to; share and sendgesture.
  Distance exclusion ranges are intersected across issuers, including the
  unbounded/off-region case; specific recipient blocks still win.
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
  double precision until their region origin is removed. Named lookups return
  a request ticket through the common action guard before teleporting, so new
  restrictions apply and superseded replies are ignored. Maturity deferrals
  also retain the issuer. Disabling cancels deferred scripted teleports.
  tplure/tprequest exceptions and accepttp/accepttprequest auto-acceptance
  retain maturity checks and recheck consent when an invitation is answered.
  standtp records the pre-sit location and returns only after a successful
  forced sit followed by a stand.
- Actions: buy, pay, rez; linkset touchall/world/attach/attachself/attachother,
  touchhud/this, interact and fartouch.
- Inventory: detach, addattach/remattach, addoutfit/remoutfit,
  sharedwear/unwear, unsharedwear/unwear. COF changes are checked as deltas;
  native detach and attach messages are checked too. Scripted removal honors
  nostrip on an item or its ancestors.
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
  extensions, legacy colour components and intensity, sun/moon angles,
  asset UUIDs and named library sky/day presets. Temporary overrides restore
  the user's local environment when released or disabled. Native environment
  changes respect the lock.
- Editing: edit, editobj, editattach and editworld at selection, local
  transforms and network writes, including material and media capabilities;
  viewtransparent and viewwireframe. viewnote/viewscript/viewtexture also
  hide already-open documents without discarding drafts.
- Script questions: acceptpermission handles only the reference's eligible
  control/attachment permissions; caution permissions and experiences keep
  their dialogs. addattach/tploc filter the corresponding permission bits.
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

`getcommand` advertises the handlers actually present. `versionnum` returns
0 and the version string identifies the experimental implementation rather
than asserting a complete RLV specification version.

## Work still required before parity

- Live-script validation of location and identity restrictions across the
  implemented UI paths, including delayed name resolutions, exceptions and
  opening an already-visible window while a restriction changes.
- Complete environment/debug conformance, wearable/attachment/sit
  notifications and permission entry-point validation.
  The new native ground-sit and clothing-notification handlers require live
  validation with delayed simulator responses and intervening restrictions.
- Complete linked/folded-folder conformance, optional shared inventory offers
  and legacy rename-on-wear settings, queue timeout
  behavior and live confirmation of attachment recovery across crossings.
- Every media/touch interaction path and surface-distance checks; all edit,
  inventory and document entry points must be audited against live scripts.
- Live visual comparison of sphere/overlay effects, forced textures,
  silhouettes and camera focus; final audit of native camera entry points.
- Protocol conformance fixtures for every registered command and every UI
  entry point, source disappearance/reattachment across region crossings,
  and human grid validation with real scripts and attachments.

Keep these items open in TASKS.md; do not close #687 or publish this as full
RLVa support. No grid connection or credentials are needed for offline tests.

## Offline validation

`cargo test --workspace rlva` exercises parsing, enable/disable, multiple
issuers, strict exceptions, redirection, UTF-8 filtering, reply channels,
camera ownership, garbage collection, shared paths and clothing locks.
Run `scripts/check.ps1` for the required complete workspace checks.

Validation on 2026-10-22, including merged PRs #805, #807, #809 and #810: `scripts/check.ps1` passes with 4,341 workspace
tests passed, zero failures and 27 ignored; scripts, formatting and Clippy
are clean. This includes 61 RLVa logic/integration/shader tests, covering
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
The additional Vulkan test
`cargo test --workspace rlva_gpu --locked -- --ignored` checks five sphere
modes, chained masks, reduced-scene upscaling and overlay alpha holes from
real GPU pixels without egui. It passes, including the extended upscale
and overlay-composition case.
The automated checks do not
validate the unfinished command families or live-grid behavior.

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
