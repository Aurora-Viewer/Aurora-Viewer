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
- Camera: setcam exclusive ownership, avatar/origin min/max distances,
  eyeoffset/scale, focusoffset, fov min/max, mouselook and unlock; forced
  offsets, fov, focus and camera mode, with distance/FOV queries.
  camdistmin/max, camzoommin/max, camunlock and touchfar compatibility names
  preserve independent removal; adjustheight and getheightoffset use the
  native hover-height path.
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
  ownership. Arbitrary settings are not exposed.

`getcommand` advertises the handlers actually present. `versionnum` returns
0 and the version string identifies the experimental implementation rather
than asserting a complete RLV specification version.

## Work still required before parity

- Location and identity filtering across every surface (showloc, shownames,
  nearby people, notifications, profiles, maps, hover tips, stored transcripts
  and application links).
- Complete environment/debug conformance (including resolution-divisor
  writes), wearable/attachment/sit notifications and permission notifications.
- Complete linked/folded-folder conformance, attachment-point groups for
  every command, shared inventory offers and rename-on-wear, queue timeout
  behavior, locked attachment restoration and reattachment across crossings.
- Every media/touch interaction path and surface-distance checks; all edit,
  inventory and document entry points must be audited against live scripts.
- Vision sphere/overlay effects, forced textures, avatar silhouettes, camera
  aliases, object focus details and every extended camera query.
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

Validation on 2026-10-21: `scripts/check.ps1` passes with 4,289 workspace
tests passed, zero failures and 26 ignored; scripts, formatting and Clippy
are clean. This includes twenty RLVa logic/integration tests, covering
environment conversions, command notifications, permission filtering,
teleport consent, edit/material/media guards, folder scopes/exceptions,
inventory moves, folded point folders, ordered outfit transactions, camera
compatibility, IM ranges and stand returns. It does not
validate the unfinished command families or live-grid behavior.

Initial native preference validation (`4e22e952`): the dev viewer builds
successfully. Inspected offline captures
`.scratch/rlva-disabled-fr.png`, `rlva-enabled-fr.png` and
`rlva-released-en.png` (1440 × 1000, frame 850): the switch is off with no
locks, on with the synthetic restrictions, then off with an empty list.
Labels, hints and session controls fit the existing Protections layout.

`AURORA_DEMO_RLVA=enabled|disabled|released` opens the native preference page
and delivers synthetic object commands through `World::apply`. Capture after
frame 800. The three modes show active locks, ignored commands and locks
released by the global switch. These fixtures do not establish grid parity.
