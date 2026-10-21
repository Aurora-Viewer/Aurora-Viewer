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
- Movement: fly, jump, unsit, sit, sitground, alwaysrun and temprun;
  forced fly, setrot, sit, sitground and unsit.
- Teleports: tplm, tploc, tplocal and sittp, with named-region or global tpto.
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
- Queries: version/new/num, getcommand, getgroup, getsitid, getstatus/all.
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

- Display-only privacy groundwork now covers the name cache, chat, hover
  text, notifications/history, radar and map labels. Canonical names and
  transcript filenames remain intact. These commands are deliberately not
  registered yet: profile, application-link, location and clipboard paths
  still need the final audit.
- Location and identity filtering across every surface (showloc, shownames,
  nearby people, notifications, profiles, maps, hover tips, stored transcripts
  and application links).
- Complete environment/debug conformance, wearable/attachment/sit
  notifications and permission entry-point validation.
- Complete linked/folded-folder conformance, attachment-point groups for
  every command, shared inventory offers and rename-on-wear, queue timeout
  behavior and live confirmation of attachment recovery across crossings.
- Every media/touch interaction path and surface-distance checks; all edit,
  inventory and document entry points must be audited against live scripts.
- Visual conformance of sphere/overlay effects, forced textures and
  silhouettes; object focus details and every extended camera query.
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

Validation on 2026-10-22, including merged PRs #805, #807, #809 and #810: `scripts/check.ps1` passes with 4,318 workspace
tests passed, zero failures and 27 ignored; scripts, formatting and Clippy
are clean. This includes 38 RLVa logic/integration/shader tests, covering
environment conversions, command notifications, permission filtering,
teleport consent, edit/material/media guards, folder scopes/exceptions,
inventory moves, folded point folders, ordered outfit transactions, camera
compatibility, IM ranges, stand returns, visual-effect lifecycle/tweens,
alpha masks, shader validation, display-only privacy, surface distances,
local teleports across region edges, encoded empty/UTF-8 replies, attachment
recovery timing, saved-asset ordering, locked/nostrip replacement, temporary
attachments, changing attachment points, allowlisted debug writes, real target
dimensions and pick-coordinate conversion. The additional Vulkan test
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

`AURORA_DEMO_RLVA=enabled|disabled|released` opens the native preference page
and delivers synthetic object commands through `World::apply`. Capture after
frame 800. The three modes show active locks, ignored commands and locks
released by the global switch. These fixtures do not establish grid parity.
