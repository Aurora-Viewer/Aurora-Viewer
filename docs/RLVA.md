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
- Movement: fly, jump, unsit, sit, sitground, alwaysrun and temprun;
  forced fly, setrot, sit, sitground and unsit.
- Teleports: tplm, tploc, tplocal and sittp, with named-region or global tpto.
- Actions: buy, pay, rez; linkset touchall/world/attach/attachself/attachother,
  touchhud/this, interact and fartouch.
- Inventory: detach, addattach/remattach, addoutfit/remoutfit,
  sharedwear/unwear, unsharedwear/unwear. COF changes are checked as deltas;
  native detach and attach messages are checked too. Scripted removal honors
  nostrip on an item or its ancestors.
- Forced wear: attach/over/all/allover, detach/all/me, remattach, remoutfit,
  through the native COF planner. Shared-folder queries: getinv, getinvworn,
  findfolder/s, getpath/new; attachment and outfit presence/name queries.
- Camera: setcam exclusive ownership, avatar/origin min/max distances,
  eyeoffset/scale, focusoffset, fov min/max, mouselook and unlock; forced
  offsets, fov, focus and camera mode, with distance/FOV queries.
- Display: showinv, showminimap, showworldmap, shownametags and
  showhovertext/all/hud/world.
- Queries: version/new/num, getcommand, getgroup, getsitid, getstatus/all.

`getcommand` advertises the handlers actually present. `versionnum` returns
0 and the version string identifies the experimental implementation rather
than asserting a complete RLV specification version.

## Work still required before parity

- Location and identity filtering across every surface (showloc, shownames,
  nearby people, notifications, profiles, maps, hover tips, stored transcripts
  and application links).
- Environment and debug extensions, restoring scripted environment changes,
  permission/teleport auto-acceptance, teleport invitation/request exceptions,
  standtp, allowidle and notifications of restriction changes.
- Complete folder-lock semantics (attachthis/except and subtree variants),
  folded folders, attachment-point groups, shared inventory offers/renaming,
  all forced-wear aliases, async originals and multi-command ordering.
- Object-specific edit restrictions and all media/touch interaction paths;
  restricted document/texture/script viewing and render mode changes.
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

Validation on 2026-10-21: `scripts/check.ps1` passes with 4,282 workspace
tests passed, zero failures and 26 ignored; scripts, formatting and Clippy
are clean. This includes thirteen RLVa logic/integration tests. It does not
validate the unfinished command families or live-grid behavior.

The native dev viewer builds successfully. Inspected offline captures
`.scratch/rlva-disabled-fr.png`, `rlva-enabled-fr.png` and
`rlva-released-en.png` (1440 × 1000, frame 850): the switch is off with no
locks, on with the synthetic restrictions, then off with an empty list.
Labels, hints and session controls fit the existing Protections layout.

`AURORA_DEMO_RLVA=enabled|disabled|released` opens the native preference page
and delivers synthetic object commands through `World::apply`. Capture after
frame 800. The three modes show active locks, ignored commands and locks
released by the global switch. These fixtures do not establish grid parity.
