# Test options and profiling

All the `AURORA_*` environment variables of the viewer: demo mode,
screenshots, test scenarios, forced settings and profiling. The command-line
arguments are in the [README](../README.md#command-line).

**Demo mode** simulates a server locally (a plaza, avatars, objects, chat,
notifications…): no connection, no account. It has its own settings and its
own cache (`…\config\demo`, `…\cache\demo`).

```powershell
$env:AURORA_DEMO = "1"
$env:AURORA_DEMO_MAP = "1"          # a scenario, if needed
cargo run -p aurora-viewer -- --title "Test my-task"
```

The logs are in `%LOCALAPPDATA%\Aurora\AuroraViewer\data\logs\`.

For localisation, launch the same scenario with `--language en`, then
`--language fr`. `AURORA_DEMO_OPTIONS=1` opens the interface preferences,
with the language selector. Remove the argument to check the immediate
change and the choice saved at the next launch. The details of the
catalogues, dates and plurals are in [TRANSLATING.md](TRANSLATING.md).

**Unit tests**: always on the whole workspace, filtered by name if needed:
`cargo test --workspace render_thread`, never `cargo test -p <crate>`. A
single crate resolves the features of its dependencies on its own, so Cargo
builds a second copy of them (`cargo test -p aurora-render` alone: 67 s of
dependencies, then again for the next `--workspace` build).

**To add a variable**: one short line in the table of its topic (the name
alone in the first column, one sentence in the second); if it takes more
than one sentence, or if there are several modes, a "Details" paragraph
under the table. The "Demo" choice of the Aurora tools reads in this file
the table rows that start with `AURORA_DEMO_<NAME>=1` and shows their second
column: it must stay short.

Contents: [Automated windows](#automated-test-windows-on-windows) ·
[General](#general) · [Screenshots](#screenshots) ·
[Camera, movement and pointer](#camera-movement-and-pointer) ·
[Objects and building](#objects-and-building) · [HUDs](#huds) ·
[Avatars and animations](#avatars-and-animations) ·
[Rendering and environment](#rendering-and-environment) ·
[Load and smoothness](#load-and-smoothness) · [Interface](#interface) ·
[Media and sound](#media-and-sound) ·
[Forced settings and diagnostics](#forced-settings-and-diagnostics) ·
[Profiling](#profiling) · [Fuzzing](#fuzzing)

## Automated test windows on Windows

For a test without human intervention, add
`[AUTOMATION - DO NOT TOUCH]` to the title and start the dashed yellow
outline. From the task's worktree, after the build:

```powershell
$env:AURORA_DEMO = '1'
$viewer = Start-Process -FilePath '.\target\debug\aurora-viewer.exe' -PassThru `
  -ArgumentList '--title "Test my-task [AUTOMATION - DO NOT TOUCH]"'
$borderScript = (Resolve-Path '.\scripts\automation-window-border.ps1').Path
$borderArgs = @(
  '-NoProfile', '-NonInteractive', '-STA', '-ExecutionPolicy', 'Bypass',
  '-File', ('"{0}"' -f $borderScript),
  '-ViewerProcessId', $viewer.Id
)
Start-Process -FilePath 'powershell.exe' -WindowStyle Hidden -ArgumentList $borderArgs
```

Use the PID of the viewer's executable, never that of `cargo`. The viewer
and the script must run in the same Windows session, on the interactive
desktop. The script uses Windows PowerShell 5.1 in STA mode.

The outline follows the position and size of the windows carrying the tag,
without taking the focus or intercepting clicks. It disappears when the
window is minimised, hidden or loses its tag, and the script stops when the
viewer's process closes. For a test with human intervention, keep the title
`Test <task>` without the tag.

## General

| Variable | Effect |
|---|---|
| `AURORA_DEMO=1` | Offline demo mode |
| `AURORA_DEMO_AVATAR_RENDER_SETTINGS=1\|empty` | Open the avatar display-exceptions editor |
| `AURORA_DEMO_POS="x,y"` | Start position |
| `AURORA_DEMO_PORTALS=1\|near\|friends` | Open the portal creator and a shared-portal preview |
| `AURORA_PORTAL_CLIP=0` | Disable avatar portal clipping for before/after comparison |
| `AURORA_DEMO_TOD=0..5` | Time of day (5: the legacy midday) |
| `AURORA_DEMO_SUN=<azimuth>,<elevation>` | Sun direction for the lighting and the shadows, in degrees |
| `AURORA_DEMO_ALTITUDE=<m>` | Puts the whole demo location at this altitude |
| `AURORA_FPS_LIMIT=<n>` | Frames-per-second cap for tests (10–500) |
| `AURORA_WINDOW_SIZE="w,h"` | Window size for this launch |
| `AURORA_RENDER_SCALE=50..100` | World resolution percentage; interface and HUDs stay native |
| `AURORA_TEXTURE_BUDGET_MB=256..32768` | Manual texture memory budget for this launch |
| `AURORA_TEST_NEUTRAL_POSE=1` | Local neutral-pose comparison in the online or demo Hover Height floater |
| `AURORA_LOG_NAME=<name>` | Name of the log file |
| `AURORA_EMOJI_FONT=<path>` | Uses another emoji font than the built-in one (PNG bitmaps: CBDT or sbix) |
| `AURORA_MAX_AVATARS`, `AURORA_MAX_COMPLEXITY`, `AURORA_AA`, `AURORA_SHADOWS`, `AURORA_PROJECTOR_SHADOWS` | Override these settings |
| `AURORA_HDR=0\|1` | Overrides "HDR and Emissive" (Graphics › Image) |
| `AURORA_GRAPHICS_PRESET=potato\|low\|medium\|high\|deluxe` | Applies a graphics level before the overrides above |

`AURORA_DEMO_AVATAR_RENDER_SETTINGS=1` opens World > Avatar render settings
with one nearby Always entry and two distant Never / Always entries.
Check live silhouette changes, filtering, Ctrl/Shift selection, changing full
detail, removing exceptions and adding residents from the shared picker.
`empty` starts with no exceptions. Both scenarios use only demo settings.

**Saved graphics profiles (#1218).** `AURORA_DEMO_BARS=profiles` installs three
demo profiles, including two with identical quality; add `AURORA_DEMO_OPTIONS=graphics` to inspect
their editor. Use `graphics:profiles-list` to open its dropdown with one bin per
profile, or `graphics:save-profile` to open the Save as name prompt. The name
field exists only in that dialog, starts focused and accepts Enter. A bin must
ask for confirmation without loading its profile, even when another is active;
all bins stay at the same right edge with long names. Cancel saves/deletes
nothing, and Save as with an existing name requires replacement confirmation.
Only the explicitly selected profile is highlighted, even with identical values.
Editing quality keeps that name and marks it modified; Save updates only that
profile without asking for a name. With no profile selected, Save is disabled
and Save as creates one. Both buttons stay beside the selector in narrow windows.
`AURORA_DEMO_BARS=fpspresets` also opens the actual FPS hover card
after frame 300; combine with `legacy` to check the other bar layout. Save a
named profile, change quality, reload it from Preferences and from the FPS
hover card, and restart the demo to check persistence of the library and selection.
Choosing a built-in level clears the named selection; deleting the selected
profile does likewise without loading another profile. Check that moving into the hover card keeps it clickable,
that ordinary FPS clicks still open Performance, and that the FPS cap toggle
retains its numeric limit. Vsync, background cap, hardware and environment are
independent of profiles. Saving while photo quality is boosted must save the
user's quality; resetting preferences must keep the named profile library.

`AURORA_DEMO_SUN=<azimuth>,<elevation>`: the direction towards the sun for
the lighting and the shadows (azimuth 0 east, 90 north), the sky keeping
its own sun. The demo camera looks east, so `0,12` puts a low sun in front
of it, which the times of day never do: the shadows come towards the
camera (the contact shadows of issue 988, see `AURORA_DEMO_FEET`).

The demo shows no sample offer or dialog by default. The fountain menu,
gift and teleport offer are restricted to `AURORA_DEMO_NOTIF=1`.
`AURORA_DEMO_DIALOG` adds only the requested dialog;
`AURORA_DEMO_INVENTORY=offer|offer-reject|offer-card…` adds only the received item.

`AURORA_DEMO_PORTALS=1` opens World > Portals with a fixed aperture four metres
ahead and its destination, owner and access label. `near` spawns it around our
avatar: standing inside and moving across it before leaving the 1 m clearance (3.5 m for another resident’s portal)
must never teleport. `friends` previews a denied friends-only portal from a
non-friend. Apertures use world depth, so walls hide them and their labels.
`AURORA_DEMO_PORTALS=return|return-private` shows a personal arrival return four metres behind the avatar, with a side view of the aperture with two additional in-memory arrival relays; all three share one aperture, including private redaction on the wire.
Ordinary creation accepts a same-grid place link or region coordinates; one
portal per creator expires after two minutes. Another Aurora client needs the
ready 1.37 bridge and scripts enabled to receive regional invitations.
Human two-client grid checks: public crossing, friends/non-friends, selected
group members/non-members, appearing on a stationary or moving avatar,
leaving and returning, crossing from either side, closing/replacing/expiry,
late arrival discovery, failed/restricted destinations and teleport cancellation.
Cross a freshly placed portal without first retreating, then use its arrival return twice. Send several travelers through the same source: destination observers must see one aperture with one unchanged expiry, including late discovery. Private destination observers see a redacted label unless authorized; Friends grants come only from the original creator, while each traveler keeps a personal return. Verify no return is created after a failed, canceled or redirected-to-another-region teleport.

`AURORA_DEMO_PORTALS=drop-friends|drop-public|drop-friends-hover|drop-public-hover|drop-invalid|drop-focus`
drags an inventory landmark over the world. Hover scenarios retain the cargo;
the others release it at frame 560. Shift changes at frame 500 in public modes:
capture around 530 for the access preview and 700 for the created portal or
the unavailable-landmark notice. Use `AURORA_DEMO_POS=100,100` to avoid plaza
props. The landmark stays in inventory. Ctrl insertion and other inventory drops
retain their normal behavior.
`drop-focus` keeps the inventory open with keyboard focus during the drag;
after release it stays open and logs that keyboard focus returned to the scene.

`drop-places|drop-places-hover|drop-favorites|drop-favorites-public|drop-favorites-hover`
uses the shared Places/Favorites landmark cargo with Places open. Hover modes
show the floating place icon/name and access hint; released modes create a portal
with its live remaining lifetime above the aperture. Pointer regressions cover
starting drags in all three Favorites access modes and both Places layouts,
including wide and filtered lists, without reordering on a world release.

`drop-history|drop-history-public|drop-history-hover` uses an actual arrival
snapshot without creating or requiring a landmark. Check its portal destination,
Friends/Shift-public access, and the unchanged left alignment of Landmarks and
History rows; reorder grips remain in Favorites only. Pointer regressions also
cover both History layouts, wide/filtered views and a visit near an owned landmark
whose exact arrival differs from the landmark's saved point.

`drop-search|drop-search-public|drop-search-hover` opens Search on Places and
drags a directory parcel without creating a landmark. Check the floating place
preview, Friends/Shift-public access and portal destination. Pointer regressions
start actual drags in both Search layouts (Places and Land) and All's Places
section even before ParcelInfoReply arrives; people and groups remain ordinary
rows. Resolver tests cover delayed parcel/map replies, immutable arrivals,
variable regions, invalid coordinates and cancellation guards.

`AURORA_DEMO_PORTALS=private-friends-allowed|private-friends-denied|private-group-allowed|private-group-denied`
exchanges real v2 codec messages between two offline peer states. Everyone sees
the aperture; denied observers see only the creator and private/access labels.
Only granted observers receive a destination. Group grants wait for a complete
server-shaped members response. On the grid, also verify late arrivals, lost
requests, revocation before a queued reply, close/expiry and bridge 1.37 upgrade.
Both creation announcements and discovery replies must omit private destinations;
inspect wire-level tests, not just labels. Old portal-v1 traffic is not accepted.

`AURORA_DEMO_PORTALS=private-friends-chat-allowed|private-friends-chat-denied|private-friends-keyboard-allowed|private-friends-keyboard-departing-allowed`
holds the actual Forward arrow from frames 600 to 810 after the private peer
exchange. Chat modes retain nearby-chat keyboard focus; keyboard mode leaves
the scene focused. Use `AURORA_DEMO_POS=100,100` and capture frame 1100
for the completed arrival (frame 840 can still show the teleport transition).
The authorized visitor must teleport in both focus modes; the denied visitor
must walk through without teleporting. Logs record focus, effective controls,
authorization, position and return journey. A refused crossing can be retried
after the ordinary 1 m clearance; the initial 3.5 m anti-drop clearance still
applies when another resident's portal appears nearby.
`keyboard-departing-allowed` starts the creator's teleport when the access
request arrives, then changes their region after the grant is sent: the
visitor must still receive the destination and teleport without chat focus.

`AURORA_DEMO_PORTALS=slice-walk|slice-run|slice-fly|slice-still|slice-large|slice-reverse|slice-wide|slice-wide-reverse`
adds an observed avatar with continuous locomotion animation, a skinned mesh
and a long rigid hand attachment. The camera observes the portal obliquely
(from the other side with `slice-reverse`). `slice-still` and `slice-large`
keep the avatar root fixed: animated limbs and worn geometry must still be
cut at the portal plane. Capture several animation phases; compare the same
scenario with `AURORA_PORTAL_CLIP=0`. Traversal never pauses or restarts an
animation. Portal clipping is visual and does not arm a teleport gate.
Use `AURORA_DEMO_POS=100,100` for a clear patch away from the plaza exhibits.
`slice-wide` and `slice-wide-reverse` keep the root still with a
4.2 m rigid attachment: only the part inside the oval may disappear, never
the parts above or beside it. The rim/corona is decorative and must not
enlarge that mask. Check body, skinned and rigid worn geometry from both
camera sides. Ordinary portals now show theme-based Aurora currents, a dark
indigo/navy core and a violet/teal rim with glow; reduced motion freezes their
animation, and denied/private observers retain muted colors and hidden details.

`AURORA_TEST_NEUTRAL_POSE=1`: exposes "Neutral pose — local test" in
Appearance › Hover Height, initially off. Our avatar starts with normal
animations; enable the switch to display the exact shaped neutral skeleton
used by automatic hover height, including worn-mesh joint offsets.
Rigged meshes and rigid attachments follow the same neutral palette.
This is local only, not saved:
the motion controller and server animation signals continue normally, and
other avatars are unchanged. Use it online to compare the reported roughly
10 cm grounding discrepancy with/without the normal pose. The option is
absent from an ordinary launch.
Hover Height exposes a saved "Automatic height offset" in every launch
(default 0 m). Positive values raise the result, negative values lower it;
the offset applies again after each body attachment change, including removal
of the last foot attachment. Millimetre numeric entry and -3..3 m adjustment
match the height control. Dragging previews immediately from the cached
estimate without another vertex scan; release saves the preference and commits
the total height. With automatic mode disabled the offset slider is inactive.
The technical measurements and recalculation button have been removed.
Use the neutral-pose test switch when comparing shoes with the ground.
The sliders share Aurora's contrasting track, zero marker, accent fill and
solid rectangular handle over the normal blurred floater. While auto is on,
the manual slider/value and Lower/Reset/Raise buttons are grey but remain
interactive. Any manual change switches auto off immediately; Reset does so
even when the height already equals zero. The saved automatic offset stays
available when auto is enabled again. Merely displaying a computed height
must not round it or switch modes.
Two shared Aurora dividers separate manual height, automatic adjustment and
the common camera/pose switches. The automatic section also has
Lower/Reset/Raise buttons: they change the saved offset, retain automatic
mode and apply the cached estimate immediately. Reset restores zero offset;
steps match the manual buttons (1 cm, Alt 10 cm, Ctrl/Shift 1 mm).
The scene compares an attachment/skeleton signature each frame, but mesh
decoding and vertex scans run only after changes settle for 200 ms. HUD,
animation and ordinary movement changes do not trigger ongoing recalculation.
Body attachment, shape and newly available asset changes can invalidate the
estimate. Its virtual standing reference comes only from the shaped rest
skeleton (pelvis-to-foot length, shape hover and mesh pelvis fixup), even in
an empty world. Terrain, platforms, world position, simulator contact planes,
streamed physical size, flight, seating and animated palettes never enter the
measurement or trigger recalculation. A mesh with missing geometry or pending
declared skin metadata retains the previous height, including failed assets.
Changing the offset reuses the cached result. Numeric `auto hover estimate`
logs expose the reference, lowest point and raw result before server limits.

The frames-per-second limiter is enabled at **120 fps by default**. It can
be disabled or adjusted in Preferences › Graphics › Smoothness; preferences
already saved are kept.

`AURORA_DEMO_ALTITUDE`: terrain, objects and avatars are raised by that much
(0 to 4000 m). Used to see what single precision does to the frame at
height: between 2048 and 4096 m, a position only varies in steps of
0.24 mm, about one pixel in a close-up.

`AURORA_GRAPHICS_PRESET` writes the quality settings of a level of
Preferences › Graphics › Presets (`graphics_presets.rs`) at start, as a
click on it would, and they are saved with the profile; the single
overrides of the table then still apply on top (`AURORA_AA=3` with a level
reads as "Custom" in the page).

`AURORA_HDR=0` turns "HDR and Emissive" off, as Potato and Low do (the
Legacy Viewer's RenderDisableVintageMode), and is saved with the profile:
no automatic or manual exposure, no tone mapping nor sharpening, the opaque
and alpha-masked PBR faces and the PBR terrain without their emissive (the
blended ones and the HUDs keep it), reflection probes clamped to 1 in 8-bit
steps. Compare with `AURORA_HDR=1` at the same place: bright skies and
highlights clip to white instead of rolling off, emissive PBR objects go
dark, and in Preferences › Graphics › Image the tone mapping, its mix and
the sharpness are greyed (the exposure too, under Light).

`AURORA_AA`: 0 none, 1 FXAA, 2 to 4 MSAA 2x / 4x / 8x, 5 TAA, 6 SMAA. Without
it, the demo keeps the setting saved in its profile: 6 (SMAA) for a new
profile, 3 (MSAA 4x) for a profile created when that was the default. MSAA
has its own multisampled depth, filled by its own prepass (terrain and
opaque faces, see `AURORA_MSAA_PREPASS`); the other modes reuse the depth of
the single-sample prepass in the scene pass, masked faces included. A change
to the vertex shaders or to the passes is therefore checked with both,
`AURORA_AA=3` and `AURORA_AA=6`: it is with the second that the terrain let
the sky show through in patches
(`AURORA_DEMO_POS="60,128"`, `AURORA_DEMO_ALTITUDE=86`).

`AURORA_RENDER_SCALE`: the world's linear resolution percentage (50–100,
100 by default), before spatial upscaling. HUDs, labels and controls stay at
native resolution. RLVa's resolution divisor takes precedence; saved photos
ignore the user scale. `AURORA_TEXTURE_BUDGET_MB` selects a manual texture
budget (256–32768 MB). Both follow the normal saved graphics settings path;
use `AURORA_DEMO=1` for isolated comparisons. A Retina comparison can use
`AURORA_WINDOW_SIZE=1728,1056`, `AURORA_GRAPHICS_PRESET=high`,
`AURORA_DEMO_TEXTURES=9000`, `AURORA_DEMO_CROWD=20` and the same capture
frame, changing only `AURORA_RENDER_SCALE` between 100, 67 and 50.

`AURORA_WINDOW_SIZE`: width and height of the render area, in logical pixels
(at least 800 × 560), to measure according to the size of the window:
without it, the demo opens at the size the window had in the last launch
that was closed, whichever agent launched it. The window is placed like that
of a screenshot (neither position nor maximised state saved) and its size is
not remembered for the next launch. The actual size is at the start of the
`perf settings` line.

`AURORA_FPS_LIMIT`: user cap for tests (10–500 fps; positive values are
brought back into this range, an invalid value is ignored). The lower cap in
the background still applies outside screenshots.

## Screenshots

| Variable | Effect |
|---|---|
| `AURORA_DEMO_JOYSTICK=<mode>` | Synthetic joystick input, preferences, lens reset gestures or an independent six-axis camera flight |
| `AURORA_CAPTURE=<file.png>` | Saves a screenshot of the frame |
| `AURORA_CAPTURE_FRAMES=<n>` | Frame captured (240 by default) |
| `AURORA_CAPTURE_EXIT=1` | Quits after the screenshot |
| `AURORA_DEMO_RENDER_TEXTURE=1` | Captures the demo world through a caller-owned render target |
| `AURORA_DEMO_XR_DEVICE=1` | Uses OpenXR-compatible Vulkan initialization for the Linux demo |
| `AURORA_OPENXR_LOADER=<path>` | Explicit macOS OpenXR loader dylib for runtime tests |
| `AURORA_DEMO_XR_WORLD=1` | Sends the native demo world to OpenXR for a short stereo test |
| `AURORA_CAPTURE_LAYOUT=1` | The screenshot reloads the saved layout (`ui_layout.ron`) |
| `AURORA_DEMO_SNAPSHOT=<scenario>` | Photo viewfinder, photo window, full-frame view, a photo saved at its size, sent to the inventory, posted to the profile or sent by e-mail, the photo quality on a card short of video memory |
| `AURORA_DEMO_SNAPSHOT_FRAME=<n>` | Frame the scenario's photo is taken (700 by default) |
| `AURORA_DEMO_SNAPSHOT_TILE=<px>` | Largest side drawn in one pass: forces the tiles |
| `AURORA_DEMO_FREEZE_WORLD=1` | With `AURORA_DEMO_SNAPSHOT=viewfinder-frozen`: a prop derezzed and particles started while the world is frozen |
| `AURORA_DEMO_ISOLATE=own\|pick[-panel][-opaque][-keep][-back]` | The photo's isolation: our avatar alone on an empty background, or chosen with a click |
| `AURORA_DEMO_PHOTO_LOOK=<look>[+<look>…][:<percent>][:compare][:panel][:tab=…][:search=…][:set=…]` | The photo's looks stacked (Legacy, Aurora or a LUT), the chosen one's strength and settings, the before / after line, the filter panel |
| `AURORA_DEMO_CONTROLS=camera\|movement\|compact\|classic\|free\|free-compact` | The Controls window open on a tab, Firestorm's two windows, or driving the photo window's free camera |
| `AURORA_DEMO_CAMERA_PRESETS=save\|apply\|object\|pick[-…]` | The viewfinder's camera presets: saved on the avatar, a place and an object, applied, or the object chosen in the scene |

`AURORA_DEMO_RENDER_TEXTURE`: with demo mode and `AURORA_CAPTURE`, the capture
frame draws the full world into a caller-owned GPU texture instead of acquiring
the desktop swapchain. The saved image must show the scene without egui,
nameplates, worn HUDs, selection or debug overlays. Other frames follow the
normal desktop path. This checks the output boundary for VR; it does not connect
to OpenXR or provide stereo views.

`AURORA_DEMO_JOYSTICK`: at frame 600, injects a synthetic eight-axis
controller through the production input path, without connecting a device
or a grid. `preferences` opens Mouse and keyboard › Joystick and 3D mouse;
`axes` filters the axis mapping rows in preference search; `avatar` walks and turns;
`flycam` starts at frame 630 and moves after 650;
`roll` also enables roll and disables auto-level. Capture after frame 900.
`rearm` keeps a relative zoom axis at -1, enters flycam with a synthetic
button press at frame 630 and leaves it at 750 while movement is held.
The other axes centre at 770 and move the avatar again from 800.
Capture at `700,760,830`: camera flight, stopped movement after leaving,
then avatar movement. The log records the flycam state and motion axes at
701, 761 and 831; the zoom channel stays zero throughout. Keep the viewer
running past frame 831 to include all three log entries.
`axis-capture` supplies minor axis-0 movement (0.05) alongside strong axis-7
movement (-0.8) during frames 700–759 and 820–879. Combine it with
`AURORA_DEMO_OPTIONS=controls/1:search=forward` (English) or
`controls/1:search=avancer` (French). Click the axis assignment before frame
700: it must capture axis 7, then re-centre at 760. Invert Avatar before
820 to see backward movement on the second gesture without changing gain
magnitude. Capture before, during and after assignment, then after inversion.
`zoom` enters flycam at frame 630 with a neutral, enabled zoom axis. Frames
650–709 apply relative zoom, frame 750 edits the lens preference to 0.9 rad,
frame 800 enables absolute zoom, and frames 820–879 deflect the zoom axis
before returning to its reference. Capture at `640,720,770,860,920`: initial
view, narrower relative zoom, edited lens, absolute deflection and restored
reference. Logs at the following frames record the rendered field of view;
keep the run beyond frame 921 to include every probe. The camera position
must remain fixed while the projection changes.
`zoom-buttons` and `zoom-triggers` enter flycam with two physical buttons
(8/9) or two analog triggers (axes 6/7) assigned to zoom. Frames 650–689
zoom in, 720–759 zoom out, and 780–809 hold both controls to reset relative zoom.
Capture `640,700,770,820,860`: initial lens, narrower lens, restored lens,
both-controls reset and retained lens. `zoom-hold` assigns trigger
6 as modifier and shares axis 5 with camera yaw. Frames 650–689 zoom in,
720–759 zoom out; 760–789 release the modifier while keeping the axis
deflected, which must not rotate the camera. Frames 790–809 centre it;
810–849 resume ordinary yaw. Capture `640,700,770,780,840,880`: fixed-pose
zoom in/out, safe modifier release, then resumed yaw. Logs at frames 641,
701, 771, 791, 831 and 861 report the lens and yaw/zoom inputs.
Append `-absolute` to `zoom-buttons`, `zoom-triggers` or `zoom-hold` to
verify position-based zoom and return to the reference at rest; it must
not accumulate with hold duration. Trigger zoom uses continuous pressure.
`reset-zoom`, `reset-zoom-buttons`, `reset-zoom-triggers` and `reset-zoom-hold`
first zoom in at frames 650–689, then reset relatively: the direct axis or
modifier gives two impulses at 730–735 and 744–769; two-control modes hold
both at 744–754, then only one until 769. The reset must hold the reference
through frame 809; new input at 810–839 must resume zoom. Capture
`640,700,750,780,850` and inspect the lens/yaw/zoom probes at frames 701,
741, 751, 781 and 841. Hold mode leaves the shared stick off-centre until
780 to verify that releasing the modifier cannot replay camera yaw.
Combine any of these with `AURORA_DEMO_OPTIONS=controls/1:joystick-zoom`
to show its normal preferences group, scrolled to the zoom mode selector.
The avatar must stay put in the two camera scenarios. Compare the photo
free camera (`AURORA_DEMO_CONTROLS=free`) and normal camera controls as well.
Hardware validation: select the real device, enable input, apply Gamepad or
SpaceMouse, centre its axes and release its buttons; check the live input,
walking, sliding, looking, jump held to fly and a press to land. Start and
stop flycam with its configured button or Alt+Shift+F, and return with Escape.
Button assignments: click the assigned button (or Assign), then press the
desired controller button, including stick clicks and buttons beyond 9.
Already-held buttons must be released and pressed again; a quick tap must
also record. The press must not move the avatar or toggle flycam. None
disables an assignment; Cancel, Escape, focus loss, a device change or
leaving the row keeps its previous assignment. Capture also works with
joystick control disabled. Zoom selects an analog axis, independently of
button numbers; it changes only the flycam's field of view. Assigning a
zoom axis changes a zero Camera gain to 1 (retaining inversion), while a
previous non-zero gain stays intact. Gain 0 still disables zoom. Absolute
zoom maps position around the flight's reference field of view; relative
zoom uses deflection as zoom speed. Lens preference edits and keyboard lens
zoom must work during flight; the absolute reference follows lens edits.
Axis assignments: click the assigned axis (or Assign) and move the intended
stick direction or trigger. Minor sideways movement must not win over the
main gesture. Detection observes up to 180 ms once movement is significant,
or finishes on release; a similarly strong diagonal waits until released
and retried. Resting triggers at -1 and small stick jitter must not assign
anything. None and cancellation behave like button capture. One Invert
switch sits beside the axis assignment and None, reversing Avatar and
Camera together while preserving each mode's gain magnitude and dead zone.
A previous negative gain in either mode appears as shared inversion;
unused Avatar gains for Roll/Zoom do not participate. The choice persists
even with zero gain and remains detectable by modified-settings/reset.
Avatar and Camera tuning use aligned columns below the assignment;
check wrapping at narrow window sizes and larger interface scales.
Roll labels the unused Avatar column as Camera only. Zoom has its own group
with direct-axis, two-buttons and hold-plus-axis choices; its axis row has
only the shared assignment/inversion and Camera tuning. Only the chosen
mode's controls appear on the page, while all remain searchable and resettable.
Assign the two zoom controls and the modifier by pressing a physical button
or trigger. Trigger/axis directions display as Axis N (+/−), not fabricated
physical button numbers. Controls held before capture must be released
before a fresh press is learned. Two-controls mode must zoom both ways,
stop when released, and work without an axis assignment or Camera gain.
With absolute zoom off, pressing both controls together must restore the
reference field of view. Hold-plus-axis mode must keep the relative lens
after release, consume the shared camera axis while held, and wait for
that axis to centre before ordinary movement resumes. A trigger used as
a zoom button or modifier must not also drive its old camera mapping.
Repeat after focus loss, a device change and stale input: held controls
must not restart zoom until released and pressed again. Old direct-axis
profiles retain their mode and absolute/relative choice.
Absolute zoom must be selectable in all three modes. Its control value sets
an offset from the flight's reference field of view; rest returns to that
reference. Two-buttons mode labels its amount as a range, rather than speed,
and trigger pressure below the button threshold must still adjust it.
In relative hold-plus-axis mode, double-press the assigned modifier; in
direct-axis mode, give two quick impulses with a return to rest between
them. The first press must release within 250 ms and the second must start
within 350 ms of the first. Both gestures must restore the reference lens,
including after a keyboard/preference edit, without moving or rotating the
camera. A long hold, slow repeats, jitter or a tap before focus/device/mode
changes must not reset it. After any reset, keep the second control held:
zoom must stay at the reference until release, then resume on fresh input.
Absolute mode must never trigger these relative reset gestures.
Windows trigger zoom: assign a trigger, let it return to rest, enter flycam
and press it. Its native pressure range must stay 0–1 (released at zero),
so relative zoom starts without crossing a midpoint and stops on release
after any configured smoothing. Repeat with Absolute zoom: release returns
to the reference lens. Check both triggers and confirm the centred sticks
still move in both directions.
Repeat the toggle while holding a movement stick, including a quick second
press. Configure zoom on an axis that rests away from zero: the other axes
and the flycam button must still work. Direct zoom also accepts a non-zero
initial position. After a focus change, release each held movement control
before reusing it; fresh buttons must work even if another axis stays held.
Check all six axes, inverted gain, zones, smoothing, focus loss, disconnect,
reconnect and saved settings. On the grid, the human verifies simulator
controls (including a seated vehicle), loading around the flycam, region
crossing and teleport. Never connect to a grid for this scenario yourself.

`AURORA_CAPTURE_LAYOUT`: a screenshot otherwise starts from the default
layout, to be reproducible. With this variable it reloads that of the
previous session (without ever writing it): this is what is needed to see
what a saved layout gives in a window of another size (the windows keep
their edge). A layout is only written by a normal exit, for example
`AURORA_DEMO_EXIT=quit` without `AURORA_CAPTURE`.

`AURORA_DEMO_SNAPSHOT`: `viewfinder` opens the photo mode in its
viewfinder at frame 600 (whatever Preferences › Aurora › Photo window
says), with the depth of field on at f/1.8 in AF-S on the Avatar area;
words joined by `-` add to it: `essential` or `hidden` the display level,
`menu`, `help`, `drawer` or `light` (the dock's light panel) a panel open, `mf` or `afc` the focus mode
(`mf` focused by hand at 3.3 m, the sharp edges outlined by the focus
peaking), `point` the Point area, `sharp` the aperture ring on All sharp (the
window's depth of field off), `prefsdof` Preferences › Graphics › Depth of
field on at f/1 and 100 mm (with `sharp`, the view stays sharp: the
Preferences' blur never comes back under the photo mode), `pad` the camera pad shown, `timer` the
2 s self-timer, `ael` AE-L on, `frozen` the world frozen (the dock's
snowflake: two captures some frames apart show the avatars, the
particles, the water and the animated textures at the same instant, the
log gives `world frozen: <n> moving objects held`; with
`AURORA_DEMO_FREEZE_WORLD=1`, a violet box and a teal disc of another
resident rez beside our avatar at frame 400, and at 650 the box is
derezzed and a script starts a fountain on the disc: with
`AURORA_CAPTURE_FRAMES=700,900` both captures show the box and a disc with
no particle, and differ by no pixel; without `frozen` the box goes and the
disc sprays), `hover` or `nocontrols` the on-screen controls
on hover or hidden (the lens ring and the AF choices show otherwise), `tilt`
the camera rolled 20° clockwise (the level turns the other way, along the
horizon), `tilt90` a quarter turn (the AF frame over a head keeps its size,
lying on its side), `poser` the poser opened over it (the dock's wand
button: its window, the joints and the gizmo drawn over the viewfinder; a
click on a joint selects it without moving the AF point), `focus`
the own avatar's face aimed at
and the shutter half-pressed at `AURORA_DEMO_SNAPSHOT_FRAME`, `shoot` the
same then released two frames later (the photo saved to disk; with `timer`
the log gives `viewfinder: self-timer, 2 s`, then `viewfinder: shutter` two
seconds after the lock, and the countdown shows at the top), `group` the
same focus then, 40 frames later, the other avatars in view within 25 m
added to it as Shift + clicks would (the log gives `viewfinder demo: <n>
avatar(s) added to the focus` and `viewfinder: group focus, <n> subject(s),
lens at <m>`; capture at frame 800 with `AURORA_FPS_LIMIT=60`: a green box
on every head, the zone from the nearest face to the farthest under the
first one and on the scale, every face sharp and the background blurred at
f/1.8). With
`viewfinder-focus` the log gives `viewfinder: focus asked at <m>` then,
about 0.36 s later, `viewfinder: locked at <m>`: capture at frame 760 with
`AURORA_FPS_LIMIT=60` to see the green box on the head, the avatar's name
over it and the distance under it,
the zone kept sharp on the scale and the beep's moment; the hunt is
between frames 702 and 722. The other scenarios below keep the photo
panel for the run.
`open` shows the photo window at frame 600;
`full` the full-frame view, `full-4x5` (or `full-16x9`, `full-9x16`…) with
those proportions, to see the mask and the guide; `save:<W>x<H>[:png|jpg|bmp][:<folder>]`
also takes a photo of that size at `AURORA_DEMO_SNAPSHOT_FRAME` and saves
it (in the photo folder, or `<folder>`). The log gives `photo: <W> x <H>
drawn in <n> part(s), <ms>`, `snapshot <W> x <H>: <MB> of textures … at its
peak` (the video memory of the photo frame) and `photo saved: <file> (<KB>,
written in <ms>)`. Open the file: its framing is the window's (cropped to
its proportions), without the interface unless asked. With
`AURORA_DEMO_SNAPSHOT_TILE=2048`, an 8K photo is drawn in 15 tiles: compare
it with the same photo in one pass, the tiles must not show (the demo's
particles and animations make a few levels of difference everywhere). Each
tile is drawn with a guard band of 128 pixels on the sides it shares with
another, its edges on multiples of 64, then cropped, so a forced side gives
more tiles than the photo divided by it: 15 tiles of 640 x 600 for
3200 x 1800 with `AURORA_DEMO_SNAPSHOT_TILE=1024`. The bloom widens the band
to 346 pixels, the depth of field to its largest blur and 66 pixels, and the
contact shadows to their rays and 3 pixels (60 pixels times the photo's
height over the viewfinder's, at most 480), at
most a quarter of the forced side: with `AURORA_BLOOM=1`, compare a
7680 x 4320 photo in one pass with the same in 6 tiles
(`AURORA_DEMO_SNAPSHOT_TILE=4096`). The glow keeps its halo in every tile
(the lamps of the plaza, the glowing objects): the same size as in one
pass, never cut along a border. With `AURORA_AO=gtao:2` and
`AURORA_DEMO_DEBUG=ao`, no dark line may cross the ground along the tiles'
borders. With `AURORA_DEBUG_SHADOWS=1`, the sun's shadows are those of the
photo in one pass, in every tile (the shadow of the box on the right of the
plaza, with `AURORA_DEMO_POS=129,134`). The world keeps the first tile's
instant until the last: with `AURORA_DEMO_SNAPSHOT_TILE=1024` at
3840 x 3840 (25 parts), `AURORA_DEMO_CROWD=12,3`, `AURORA_DEMO_PHYSICS=1`
and `AURORA_DEMO_POS=129,134`, the hopping avatars are never cut along the
tiles' borders.
`inventory[:<side>|window]` opens the window on the Inventory destination
(512 by default), asks the price at `AURORA_DEMO_SNAPSHOT_FRAME` and
confirms it 60 frames later: the texture is encoded, the demo answers the
upload, the inventory opens on the photo in the Photo Album and the balance
loses L$ 10 (the log gives `photo: texture <W> x <H> encoded in <ms>` and
`photo: saved to the inventory`). `inventory-ask` leaves the price's
confirmation on screen; `inventory-poor` sets the balance to L$ 5 and shows
the "cannot afford" message instead.
`profile` opens the window on the Profile destination with a caption and
posts the photo at `AURORA_DEMO_SNAPSHOT_FRAME` to a local web profile that
answers like my.secondlife.com and its storage (session cookie, upload
configuration, form, 303): the log gives `demo profile: photo posted (<KB>
PNG, caption "…", location true)` and the window "Posted to your profile".
`profile-fail` has the configuration refused: the window shows the failure.
`postcard` opens the window on the E-mail destination, a recipient and a
message filled in, and sends the photo at `AURORA_DEMO_SNAPSHOT_FRAME`: the
JPEG is encoded (log `photo: postcard of <n> bytes`), nothing leaves the PC
and the window says "E-mail sent!"; `postcard-fail` answers as a server
refusing a photo too big.
`dof` opens the window with its depth of field on (f/1.4, focused by hand
at 3.3 m, the own avatar), `full-dof` the full-frame view with the aperture
in its strip: the view, the preview and the photos are blurred, while
Preferences › Graphics › Depth of field stays off. `prefs-dof` turns
Preferences › Graphics › Depth of field on (f/1, 100 mm) and leaves the window's own off
(All sharp): the view, the preview and the photos stay sharp.
`quality` opens the window with its "Photo quality" on: the log gives
`photo quality: on (draw distance 512 m, shadows 4, LOD 4, MSAA 8x)` and the saved
settings keep the user's own values (`draw_distance` unchanged in
`settings.json` after the run); a photo taken then logs
`photo quality: details loaded in … ms` and is drawn at twice its size
(`photo: reduced to …`); `save-quality:<W>x<H>…` takes such a photo like
`save`.
`quality-tight[:<MB>[:<MB later>]]` opens it the same on a card short of
video memory: the photo quality reads the driver's budget as what the
viewer held when it came on plus `<MB>` (900 by default), then plus
`<MB later>` from `AURORA_DEMO_SNAPSHOT_FRAME`. It lowers itself to fit,
with a tenth of the budget (at least 512 MB) kept free and more as the
textures are pressed: the MSAA first (8x, 4x, 2x, then the user's own
anti-aliasing), then the draw distance (384, 256, 128 m, then the
user's), then the shadows, then everything else, never below the user's
own settings (`snapshot::quality::plan`), and the textures in view are no
longer forced to full resolution. The panel shows "Quality reduced: video
memory almost full" under the Resolution section (the viewfinder says it
once at the top), and the log gives `photo quality: on (…, lowered for the video
memory)` then `photo quality: step <k> of <n> for the video memory, about
<x> MB beyond the user's settings (…)`. What the driver counts grows
by more than the estimate when the render targets are rebuilt (the free
room of the allocator's blocks), so the first step often settles one or
two lower (the default: step 3 of 7, then 5). The window stays open and the
memory is read again every second: `quality-tight:2000:100` starts with
the whole boost and lowers it at frame 700 (`photo quality: lowered`),
`quality-tight:100:3000` starts lowered and climbs back one step every
five seconds (`photo quality: raised`), the line gone at step 0. With
enough room (`quality-tight:5000`) nothing changes from `quality`.
`light` opens the window with its quick light set: sunset, exposure 1.6,
the clouds still and the worn lights off (the log gives `photo light:
PhotoChoice { … time: Some(3) … }`); the saved `time_of_day` and
`exposure` stay the user's.
`windlight` opens the window on its Light section with the library's
"Coastal Sunset" sky, "Murky Water" and the sun turned low in the east
(azimuth 80°, elevation 12°), for the photo only: the lists show them, the
environment window keeps the user's own. `windlight-keep` then presses
"Keep this light" (frame 900, the log gives `photo light kept as the local
environment`) and closes the window (960): the scene keeps that light.
`classic` shows Firestorm's floater instead of the panel for this run
(Preferences › Aurora › Photo window, not saved): a floating window in the
middle, Refresh and the controls on the left, the preview on the right.

`AURORA_DEMO_ISOLATE` (with `AURORA_DEMO_SNAPSHOT`, which opens the photo
mode): at frame 640 `own` shows our avatar alone (My avatar), the sky,
terrain and water left out: the window shows a checkerboard around it, and
a photo saved then (`save:<W>x<H>:png:<folder>`) is an RGBA PNG, empty
around the avatar, its edges and the blended faces partly transparent
(`AURORA_DEMO_CLASSIC=1` with `AURORA_DEMO_HAIR_CARDS=24,layered` gives
strands to look at). `-opaque` keeps the sky and the ground. `pick`
chooses in the scene instead (My choice): the pointer at the middle of the
window, 70 % down (our avatar in the viewfinder), from frame 640, its name
beside it, a click there at 650 (outlined from then on), and the choosing
stopped at 680, unless `-keep`: the log gives `photo isolation: <id>
chosen` and `isolation demo: choosing stopped, 1 chosen`. `-panel` opens
the viewfinder's isolation panel. Use `AURORA_FPS_LIMIT=60`
and capture from frame 700 (`AURORA_AA=3` and `6` for the two edge paths,
MSAA and SMAA). `-back` returns to the entire scene at frame 1500: with
`AURORA_DEMO_STREAM=3000,3000`, `AURORA_VRAM_BUDGET_MB=2048` and
`AURORA_PROFILE=1`, the `memory summary` lines show that the textures of
what the isolation leaves out are only lowered as out of view under memory
pressure (`texture_mb` about 110 while isolated, not down to a few MB), so
the entire scene does not reload all at once on its return (#1374).

`AURORA_DEMO_PHOTO_LOOK` (with `AURORA_DEMO_SNAPSHOT`, which opens the photo
mode): at frame 600 the photo takes those looks, stacked in the order
given with `+` (`Sepia+aurora:fine-grain`), the last one chosen: each a
Legacy Viewer filter by
its file's name (`Sepia`, `Toycamera`, `Lightleak`, `Miniature`,
`Posterize`…, see `crates/aurora-render/src/looks/legacy/`), one of Aurora's
looks as `aurora:<id>` (`aurora:instant`, `aurora:vhs`, `aurora:haze`…, see
`crates/aurora-render/src/looks/aurora/`), a table of the library as
`lut:<file>`, or `lut:demo` (a warm table, "Demo warm", written into the
demo's library); the chosen one at 100 % or the percentage given, its
settings set with `set=<v>,<v>,…` (an Aurora look's values in the order of
its sliders, `aurora:glitch:set=0.03,0.02,42`); the log gives `photo look
demo: <look> at <n> %` for each. `compare` puts the before / after line in
the middle of the viewfinder's frame (the scene as it is on its left,
"Before" and "After" over it); `panel` opens the viewfinder's filter panel
on the filter's tab (`tab=legacy|aurora|luts` for another), or scrolls the
photo window to its Filter section; `search=<text>` types a search in the
panel (`search=grain` lists Fine grain, Silver black and white and Night
vision). The depth looks (`aurora:haze`, `aurora:miniature-3d`,
`aurora:subject`) take the viewfinder's focus: with `viewfinder-mf` the
focus is at 3.3 m. Without the variable, the photo scenarios start with no
filter. With `viewfinder`, the whole frame shows the filter live; with
`open` or `classic`, the view and the preview do; with `save:…`, the file
is drawn with it (`Posterize` with `AURORA_DEMO_SNAPSHOT_TILE=1024`: the 15
tiles share the histogram curves, no seam along their borders). Capture
from frame 700.

`AURORA_DEMO_CAMERA_PRESETS` (with `AURORA_DEMO_SNAPSHOT=viewfinder`, steps
joined by `-`): the presets panel opens at frame 600. `save` saves three
presets at 610: "Close up" (on the avatar, the position only), "Plaza" (the
place, the lens and the light) and "Bench" (the object nearest the avatar,
every part); the log gives `camera preset saved: …` with the parts kept and
the camera's position. `apply` turns the camera away at 620 and applies
"Plaza" at 660: the log gives the camera at 700 at the position it was saved
at, and a capture from frame 720 shows the first view again (capture at 640
for the view turned away). `object` applies "Bench" instead. `pick` chooses
the anchor object in the scene: the panel on Object, the pointer at the
middle of the window, 70 % down, from 630 and a click there at 640 (the log
gives `camera preset: anchor object <id>`); capture at 636 for the choosing
button lit. The presets are saved in the demo profile's settings, like the
Controls window's.

`AURORA_DEMO_CONTROLS` opens the Controls window at frame 600: `camera` on
its Camera tab (joysticks, zoom, the four views, object view, mouselook,
reset, saved positions), `movement` on its Movement tab
(the eight buttons with the keys of the keyboard's layout, walk / run / fly),
`compact` the window in its compact view (its joysticks and zoom only),
`classic` Firestorm's camera and movement windows (Preferences › Aurora,
not saved). `free` opens the photo window with its "Free camera" switch on:
the Controls window opens by itself on its camera tab, which drives the free
camera (the "Free camera / The avatar stays put" banner with Back and its
key, the move and look joysticks with up / down and their keys between them,
the speed at 2 m/s, "No collisions" on, the keys); at frame 640 the camera
has flown 2 m to the left and 1 m up and turned, the avatar where it was
(the log gives `free camera: on at …` and `demo free camera: flown to …`).
`free-compact` is the same window minimized: its title is "Free camera", the
two joysticks with up / down, the speed and Back. Escape, Back or closing the
photo window brings the camera back (`free camera: back to the avatar`).

`AURORA_CAPTURE_FRAMES`: allow about 620 to get past the loading fade.
Several frames separated by commas (`2500,2600`) save one file each,
`<file>-<frame>.png`. The frames-per-second cap in the background ("Limit
when unfocused") is ignored during a screenshot.

## Camera, movement and pointer

| Variable | Effect |
|---|---|
| `AURORA_DEMO_CAM="yaw,pitch,dist"` | Places the camera around the avatar |
| `AURORA_DEMO_CAMERA=<scenario>` | Camera scenario (see Details) |
| `AURORA_DEMO_CROSSHAIR=dot\|cross\|circle\|legacy\|off\|image:<file>` | The mouselook crosshair in that shape for the session (see Details) |
| `AURORA_DEMO_HOVER_HEIGHT=<scenario>` | Manual/automatic avatar grounding and camera toggle |
| `AURORA_DEMO_TP=1` or `"x,y,z"` or `remote` | Teleport after loading |
| `AURORA_TP_SCREEN=legacy` or `alt` | The teleport screen of before 2026-10-17, to compare |
| `AURORA_DEMO_TP_AWAY=1` | Departure out of range of the demo region, then return |
| `AURORA_DEMO_AGENT_JITTER=<mm>` | Small position corrections of our avatar |
| `AURORA_DEMO_LINK=<ping ms>[,…]` | Our avatar moved by a stand-in simulator through a simulated link, to compare Aurora's movement with Firestorm's |
| `AURORA_DEMO_KEY=<arrow>[:pulse]` | Holds an arrow key (or presses and releases it, over and over) |
| `AURORA_DEMO_SIT=<n>` | Clicks n times on the Sit button |
| `AURORA_DEMO_SIT_TP=local\|region` | Sits on the nearest prim, then teleports from the seat |
| `AURORA_DEMO_MMO="x,y[,flags]"` | Mouse steering and optional world walking with both buttons |
| `AURORA_DEMO_DOUBLECLICK=walk\|run\|tp[,x,y]` or `menu` | Double click on the ground, its wolf menu entry |
| `AURORA_DEMO_AVATAR_TRAVEL=<mode>` | Avatar navigation menu, approach, persistent follow and automatic flight |
| `AURORA_DEMO_RCLICK="x,y"` or `tag` | Right click (context menu) |
| `AURORA_DEMO_PIE=1` | Right-click pie menu |
| `AURORA_DEMO_TEXT_MENU=1` | Right-click menu of a text field (Cut, Copy, Paste…) |
| `AURORA_DEMO_MENU_SEARCH=<query>` | Wolf menu filtered by its search field |
| `AURORA_DEMO_POINTER=<points>` | Moves, clicks or holds the pointer over the interface |
| `AURORA_DEMO_HOVER=1` or `"x,y"` | Cursor swept, or resting on a pixel, on hover |
| `AURORA_DEMO_CLICKS=<ms>` | Clicks on the world at a regular rate |
| `AURORA_DEMO_LOOKAT=1\|own[,x,y]\|targets\|leave[,x,y]\|ahead` | Gaze tracking and gaze of another avatar; `own` / `targets`: the look-at crosshairs; `leave`: an avatar looking at us logs off; `ahead`: our eyes come back straight when the look-at is turned off |
| `AURORA_DEMO_POINTING=1\|release\|private\|remote\|remote-release\|aim` | Editing arm and gun targeting, driven by local selection or received point-at effects |
| `AURORA_DEMO_KEYBOARD=<layout>` | Default movement and letter shortcut keys according to the layout |
| `AURORA_DEMO_CHATCMD="<commands>"` | Types commands in the chat bar |

### Details

- **`AURORA_DEMO_CROSSHAIR=dot|cross|circle|legacy|off|image:<file>`**: the crosshair of
  Preferences › Camera › Mouselook › Crosshair in that shape for the
  session, nothing saved (`cross` and `circle` with their centre dot; `off`:
  none, as with "Show the crosshair" off; `image:<file>`: that picture, as
  if chosen with "Choose image", 16 px on its longer side, a dot while it
  loads or if it cannot be read, with a warning in the log). Combine it with
  `AURORA_DEMO_CAMERA=ml` and a capture at frame 650: the mark sits exactly
  in the middle of the window, white with a dark outline; `legacy` shows the
  Legacy Viewer's four corners (crosshairs.tga) at twice their size (the
  default 16 px). The preferences' preview shows the same mark over a dark
  and a bright patch: `AURORA_DEMO_OPTIONS=camera/2:crosshair` opens the
  group and scrolls down to its last row, the image.
- **`AURORA_DEMO_AVATAR_TRAVEL=<mode>`**: at frame 600, `menu` opens
  the violet wolf's avatar menu; the other modes dispatch Go to or Follow. `go`
  keeps the initial destination and ends within 2 m; `follow` keeps the
  leader, rests near it, then resumes after the wolf moves 6 m at frame 1500
  (the wolf also moves in `go`, but the pilot stays finished).
  Follow opens its compact controller: change the distance while following,
  enable/disable teleport recovery, then Stop or close it (both end the follow).
  The defaults are 2 m and stop on an obstacle; Go to opens no controller.
  `controls-distance` starts at 6 m. `stuck-stop`, `stuck-teleport` and
  `stuck-denied` hold our body still to simulate an obstacle: capture at 1200
  or later. The first ends the follow, the second teleports beside the precise
  target and resumes, and the third enables RLVa teleport restrictions and
  must stop without teleporting. A failed teleport cancels recovery; a successful
  arrival that is still blocked cannot request another teleport until the
  follower has actually travelled at least 1 m. Recovery waits at most 30 s
  for arrival/target loading, and movement, Escape, Stop or closing cancels it.
  `follow-menu` shows the active follow's menu at 1400. `manual`, `escape`,
  `stop` and `lost` start a follow, then cancel it at 1500 through the movement
  controls, Escape's reset action, Stop following or target removal respectively.
  `flight` follows the wolf through takeoff at 900, landing at 1500, a
  second takeoff at 2100 and landing at 2700. Capture at 1400, 2000, 2600
  and 3200; flight must follow the leader and remain active after each landing.
  `go-flight` starts Go to with an already-flying target and ends nearby in flight.
  `go-flight-late` delays the hover animation until frame 620, after Go to
  started at 600: it must still take off while keeping its fixed destination.
  `run` follows a running target, walking at 1200 and stopping at 1500;
  milestone logs show temporary running enabled, then restored. `flight-brake`
  approaches a hovering target 35 m farther away. Combine it with
  `AURORA_DEMO_LINK=300,jitter=40,aurora,native-flight` or
  `300,jitter=40,firestorm,native-flight` and
  capture after frame 4400: the follower must stop beside the target at its
  height after one continuous burn per axis, with no final approach impulses
  or repeated crossings. The delayed native demo exercises the network/prediction
  path; `native-flight` adds full-speed flight and inertia to that simulator.
  Use `AURORA_DEMO_LINK=0,native-flight` with the other flight scenarios too.
  The deterministic regression additionally covers different frame rates,
  short/high-latency approaches, ascent/descent, height alignment and one burn
  per axis. A nearby stationary hover must remain still; a leader moving away
  horizontally or vertically must restart the follow.
  Logs include our flight state, landing phase, altitude and height error. A stationary
  hover must cause takeoff; jump/fall or missing signals must not cause a
  spurious flight change. Takeoff respects region, parcel and RLVa permissions.
  Capture the menu at 620 and the final follow at 2450; milestone distances,
  active state and control flags are logged. With the native menu, verify
  Rejoindre/Go to, Suivre/Follow and Arrêter de suivre/Stop following in both
  languages, on the avatar and its worn objects, and in resident lists.
  On the grid, the human checks a moving resident, stable arrival, a target
  sitting on an object, manual movement/Escape, target departure, a region
  crossing and flight changes: start on the ground, let the leader take off,
  hover, land, walk and take off again; repeat on a raised platform. We descend
  near the landed leader and clear flight only on our own physical support,
  never merely on the leader's landing signal or our predicted arrival.
  Repeat Go to on an airborne resident, follow a runner who walks/stops,
  then approach a stationary flyer from 30 m or more: our avatar must brake
  once beside the target at the same height, then remain there without small
  approach impulses or passing back and forth through the target.
  These actions do not route around walls.

- **`AURORA_DEMO_HOVER_HEIGHT=panel|<metres>|cycle|camera|unsupported|failure|auto-static|auto-static-off|auto-rigged|auto-contact|auto-offset|auto-state|auto-pelvis|auto-stilt|auto-cycle|auto-ik|auto-ik-moving|auto-ik-others|sit-ground`**:
  opens the hover-height floater at frame 600. A number sets the height;
  `cycle` previews +0.4 m at 720, commits at 780 and resets at 960. `camera`
  repeats the cycle with "Affect camera view" enabled; otherwise the camera
  stays at its original height. Capture at 660, 750, 840 and 1020; include
  719, 720 and 721 to catch a camera jump on the first adjusted frame.
  With the toggle off, drag continuously in both directions: the background
  must stay fixed throughout, also when seated or using a scripted camera.
  `unsupported`
  disables adjustment as on a region without AvatarHoverHeightEnabled;
  `sit-ground` turns on a separate +0.5 m height for sitting on the ground at
  600, then presses the sit shortcut (Alt+Shift+S) at 660 and again at 900:
  the floater follows to its "Sitting on the ground" tab, the avatar sits
  0.5 m above the plaza (capture at 780), then stands back at its standing
  height (capture at 1000).
  `auto-ik` enables Aurora placement with rigged shoes, then disables automatic
  height at frame 720 and enables it again at 840 without editing manual
  height. Capture at 700, 800 and 900: soles must keep their floor contact.
  `auto-ik-moving` also enables placement while walking; walk and stop to
  verify the same height with automatic adjustment on and off.
  `auto-ik-others` puts a second resident wearing rigged shoes beside us,
  with a 28 cm mesh pelvis-rest offset replaced by an absolute animation.
  Its simulated height already places the shoes on the floor. Capture at
  700/800/900: Aurora placement is OFF/ON/OFF, while our automatic height
  also switches at 720/840. Both avatars must retain floor contact and
  their upright posture, without an extra pelvis drop or knee flexion.
  `auto-ik-perf` / `auto-ik-perf-off` place the crowd around our avatar, fix
  the same camera, enable 100 full avatars and compare Aurora placement ON/OFF.
  Use `AURORA_DEMO_CROWD=200`, `AURORA_DEMO_FEET=opaque`,
  `AURORA_IMPOSTOR_POSES=every`, `AURORA_PRESENT_MODE=immediate`,
  `AURORA_FPS_LIMIT=500` and `AURORA_PROFILE=1`. Run the ON scene with
  `AURORA_FOOT_IK_SCOPE=all` and `visible10`; the latter selects at most ten
  other unseated residents intersecting the main camera frustum, nearest to
  our avatar (not the camera). Our own avatar keeps correction. Calibration
  follows the same budget, with existing 0.5 s pose release on deselection.
  This override only applies to offline demos. Normal viewers use the saved
  maximum in Preferences > Avatars > Animations (foot-placement cog):
  five other avatars by default, 0-100, with zero meaning self only. Older
  preferences without the new key acquire five; explicit saved values persist.
  `foot scope` log lines report selected/cached remote residents and support
  frames (including our avatar); verify the cap alongside timing results.
  Crowd foot-placement A/B (2026-10-26, source `92a040319`, optimized dev):
  200 extra residents around us, 202 skeletons posed every frame, 1600x900,
  immediate presentation, 500 FPS cap. Three interleaved 45-second runs per
  mode; discard startup/calibration, retain the last fifteen one-second
  summaries at least eight seconds after the fixture is ready. Values below
  are medians of the three per-run means (45 summaries per mode).

  | Aurora placement | FPS | Frame ms | Avatar stage ms | GPU ms |
  | --- | ---: | ---: | ---: | ---: |
  | Off | 179.8 | 5.566 | 1.196 | 3.871 |
  | All residents | 35.6 | 28.079 | 23.041 | 3.764 |
  | Visible nearest ten + self | 137.9 | 7.253 | 2.734 | 3.713 |

  All nine runs have the same render counts: 100 full models (35 in view),
  102 impostors (74 in view), one silhouette and zero loading avatars.
  Selection telemetry confirms 201 remote calibrations/202 support frames
  for unlimited placement, ten/eleven for the limited experiment, zero/zero
  when disabled. The limited mode uses about 88.1% less avatar-stage time
  than unlimited placement and delivers about 3.87x its FPS in this fixture.
  Across runs, FPS ranges were 176.2-212.3 off, 35.4-40.5 all and 136.5-141.2
  limited. One to three other compiler processes were observed at run start:
  this is a shared-desktop comparison, not an isolated solver benchmark.
  The avatar stage includes pose evaluation and calibration, not only IK.
  Captures were inspected in all three modes; runtime logs have no errors or
  panics before the scheduled process stop. Those results describe the historical source before the saved five-avatar
  default. Real outfits, camera transitions and grid behavior still need human validation.

  `auto-ik-budget` exercises the saved control with no benchmark override:
  five at frame 600, zero at 900, twelve at 1100 and five at 1300. Combine
  with `AURORA_DEMO_OPTIONS=avatars/3:cogs`, `AURORA_DEMO_CROWD=60`,
  `AURORA_DEMO_FEET=opaque`, `AURORA_PROFILE=1` and capture 700/1000/1200/1400
  in English and French. The control must be integral, enabled with Aurora
  placement, searchable and reset to five with the Avatars page. Calibration
  and support-frame counts must follow the saved limit; self stays eligible
  at zero. On the grid the human checks live changes, camera turns and restart
  persistence on their real outfits.
  Acquisition regression: begin with a resident outside the saved budget,
  then increase the maximum. Also turn the camera away and back while its
  release is still running. The first corrected frame must retain the visible
  pose, followed by a roughly half-second transition, including shoes that
  were initially below the surface or tilted. A new sole measurement during
  release must not reset that transition. Automated cases exercise 30/60/144
  FPS, live animation, final contact, and the real budget/camera/calibration
  path. For a native sequence, use `auto-ik-budget` with 60 crowd avatars and
  capture 1100/1124/1132/1144/1160/1200 without the options panel; twelve residents
  become eligible at 1100 and acquire their correction after calibration.
  Automated cases measure continuity, segment lengths and rendered sole contact.
  Broader outfit/AO coverage remains tracked under #667.
  `failure` simulates a rejected update and exposes Retry. Demo changes stay
  in memory and never contact a grid. Use the slider, its editable value,
  +/- (Alt: 10 cm, Ctrl/Shift: 1 mm, otherwise 1 cm), Reset and the camera
  switch for the manual check. On the grid, also verify other viewers see
  the committed adjustment, then teleport and reconnect to check persistence.
  For account isolation, set different Standing / Sitting on an object /
  Sitting on the ground heights and separate-height switches on two accounts.
  Also give them different automatic ON/OFF, automatic offset, Affect camera
  view and foot grounding preferences. Alternate accounts, then restart the
  viewer: each must restore all its own choices. Resetting preferences on one
  account must leave the other's height options intact. Former shared options
  migrate only to the first account used after the update; new accounts start
  with defaults, including automatic mode OFF.
  The avatar's Appearance › Hover Height entry must remain available after
  changing accounts, even while region features are pending: automatic and
  camera options can still be saved. Manual height controls require the region's
  advertised support; server writes also wait for AgentPreferences. Transient
  SimulatorFeatures failures must recover without another login; an explicitly
  unsupported region must still block manual edits and all height writes.
  `auto-static` adds two rigid soles with a linked heel at frame 620;
  the `auto-*` scenarios frame the feet at 600, after the login camera reset.
  Their avatar deliberately reports a 1.68 m streamed size; the fixture places
  its centre at half the shape-derived physical box height above the plaza,
  replacing the general demo simulator's fixed 0.84 m clearance. Calibration reconstructs the standing physical
  box from shape body height/hover, including SL bounds and physical padding,
  then applies the neutral skeleton's rendered root offset. It never reads
  streamed physical size, world geometry or contact distance.
  `auto-static-off` repeats it with automatic adjustment disabled for comparison;
  `auto-rigged` adds a rigged shoe mesh on the spine with an unrelated low
  triangle whose foot weights are zero (it must not lift the avatar).
  `auto-contact` uses those shoes with the same physical size while injecting
  absent contact through frame 719, a plane 40 cm above the actual floor at
  720, a cleared airborne plane at 840 and a plane 30 cm below the floor at
  960. Every stage must retain the same height as `auto-rigged`. Enable
  `AURORA_TEST_NEUTRAL_POSE=1` to compare the exact neutral pose.
  `auto-offset` uses that same contact-plane fixture: +0.05 m offset at 700,
  then -0.03 m at 860, relative to the unchanged automatic baseline. Removing
  the shoes at 940 leaves the fixed -0.03 m offset even with a cleared contact
  plane. Capture 700, 820, 920 and 1100.
  `auto-state` resets/disables auto at 700, starts flying with a deliberately
  inconsistent 12 m physical size at 720 and enables auto at 740; it resets
  again at 840, stops flying, enables auto at 880 and changes physical size
  to 0.5 m at 960. Captures at 660, 820, 920 and 1100 must show the same
  baseline, whether enabled standing or flying and after the size change.
  `auto-pelvis` holds the animated pelvis at zero while moving the mesh rest
  pelvis to +10.53 m at 720, -8 m at 840 and zero at 960. Captures at 660,
  820, 920 and 1100 must retain the same rendered feet and raw hover result.
  The numeric log's neutral pelvis, reference and lowest foot point move
  together; their difference must stay unchanged and never saturate at +/-3 m.
  `auto-stilt` first wears the rigged 18 cm soles, then attaches a single
  60 cm box below the left foot at 720, resizes/repositions it to 1.2 m at
  840 and removes it at 960. Capture 660, 820, 920 and 1100: the raw heights
  must reflect the full 0.60 / 1.20 m stilt depths relative to the same neutral
  foot, then return to the shoe baseline, keeping the lowest sole/stilt vertex
  on the plaza. Numeric rigid-surface logs expose the measured box size,
  neutral transform and lowest vertex for comparison with on-grid reports.
  `auto-cycle` adds the rigid shoes at 620 and removes them at 940; capture
  580, 840 and 1100 to compare barefoot / soles on the ground / barefoot.
  Automatic adjustment is off by default in Hover Height; saved choices are
  preserved. The `auto-*` scenarios enable it explicitly except
  `auto-static-off`. Manual height edits disable automatic adjustment;
  automatic offset edits keep it active. HUD changes must leave the height
  intact. Calibration and offset editing work while flying or seated.
  On the grid: enable auto on the ground, in flight and seated with the same
  outfit and offset; check the same result. Add/remove rigged shoes/body at
  the hand, spine or nose, rigid foot/shin boots and linksets, and HUDs,
  including delayed skin/mesh loading and
  attachments worn while seated.

- **`AURORA_DEMO_CAM`**: heading offset and pitch of the camera around the
  avatar (radians, a positive pitch looks down) and distance (metres).

- **AltGr shortcut checks (#1371)**, on a keyboard layout with AltGr: in
  Preferences › Shortcuts, bind an action to AltGr + F9, then AltGr + Shift +
  F9 in its other slot. They must display Ctrl + Alt + F9 and Ctrl + Alt +
  Shift + F9, report the same conflicts as those combinations, survive a
  restart and run the same actions with either spelling. Outside a text
  field, AltGr + Up must orbit the camera vertically as Ctrl + Alt + Up;
  AltGr + Shift + an arrow pans. Existing Ctrl + Alt bindings stay intact.
  With chat, a script/notecard editor, a login field or a web/media field
  focused, type the layout's AltGr characters (`@`, `€`, braces, backslash):
  they must type normally, without opening a viewer window, changing a
  shortcut's key label or moving the camera. AltGr + mouse drag keeps its
  orbit and pan; AltGr alone in mouselook must not free the cursor.
  AltGr + F9 still runs its shortcut with a text field focused. In the
  build tools and photo viewfinder, a shortcut on AltGr + G must not also
  trigger plain G (snap / guide). Releasing a key first pressed without
  AltGr must always release it, even if AltGr was held in the meantime.

- **`AURORA_DEMO_CAMERA=alt,x,y|altgr,x,y|pan,x,y|zoom,x,y|crouch-alt,x,y|crouch-pan,x,y|crouch-zoom,x,y|close,x,y[,n]|drift,x,y[,n]|zoom-resident|tag|ml|ml-alt|ml-zoom|ml-zoom-alt|wheel|fly|sit|sit-ml|sit-ml-reclined|scripted|scripted-sit`**:
  scenario logged under `demo camera` — Alt+click at (x, y) then drag and
  return while walking; `altgr`: the same orbit with AltGr alone held, as
  winit reports it (neither Ctrl nor Alt), so its `demo camera` lines match
  those of `alt` frame for frame; `close`: Alt+click at (x, y) at frame 600, zoom for
  n frames (60 by default), then the camera no longer moves (close-up with a
  fixed camera, to compare consecutive screenshots); `drift`: the same
  close-up, then a very slow orbit (one fiftieth of a mouse pixel per frame)
  until frame 1200; press on our own name plate then drag (steering);
  entering / leaving mouselook (frame 650 shows "Press Esc to return to
  world view" above the bottom bar); `ml-zoom`: the right button held in
  mouselook at 640 (the normal angle stays: no zoom was set with the wheel
  yet), 3 wheel clicks in at 650 (`fov` in the log drops by 17.2° at once),
  released at 680 (the normal angle again, exactly), held again at 720 (the
  zoom at once), released at 760, a wheel click out at 800 (leaves mouselook);
  `ml-zoom-alt`: Alt + right click in mouselook at 640, no context menu
  unless Preferences › Camera › Mouselook › "Context menu with Alt + right
  click" is on; `ml-alt`: Alt held in mouselook from 620 (the cursor shown
  at the centre, no crosshair, `alt free=true` in the log) with mouse motion
  from 625 to 640 (the view stays: same yaw at 622 and 641), Alt let go at
  650 (`cursor held=true` again) and the same motion until 670 (the view
  turns); wheel; pulling back in flight; sitting on a
  seat that rotates, with a seat camera and an animation started by a linked
  prim (frame 300), then standing up and a request to stop it (frame 700;
  `camera/demo.rs`).
  `crouch-alt,x,y`, `crouch-pan,x,y` and `crouch-zoom,x,y` enable toggle
  crouch, press Down at frame 560 and release it at 570 before the same
  focus/drag at 600–660. After releasing the mouse at 661, the focus and
  chosen orbit/pan/zoom must remain until walking at 700 returns the camera
  to the avatar. Capture at 690 and 780; compare the ordinary `alt`, `pan`
  and `zoom` scenarios. The crouch control must remain active throughout
  (`down=true` in the log); add `AURORA_DEMO_LINK=0,firestorm` to exercise
  the crouch-walk through the offline simulator too.
  `zoom-resident` runs People › Near you's Zoom (the double click) on the
  nearest standing resident at frame 600: the camera faces them, 2.5 m away.
  The resident then slides 1.5 m south (660–760) and 1.2 m west (761–860).
  Capture at 655, 760 and 900: the same view of the resident each time, the
  scene moving behind them; `target` in the log moves with them.
  `sit-ml` and `sit-ml-reclined` use that seat upright or tilted by 1.1 radians,
  delayed 400 frames past the loading fade. First person runs from 760 to 1060;
  mouse motion looks up at 880–900, down at 950–970 and sideways at 1020–1040.
  Capture at 860, 920, 990 and 1050:
  the camera must follow the animated eyes without orbiting around the head;
  returning to third person at 1060 restores the seat camera, then standing at
  1100 restores the normal rear view.
  `ml-body-on` / `ml-body-off` set the own-avatar preference for this demo,
  enter first person at 600, look down at 620–640 and leave at 700. Capture at
  650 and 740. The enabled body and yellow chest attachment stay visible below
  the camera; the system head, red skull/Jaw attachments and linked skull child
  are absent from the direct view. With the preference off, the whole own body
  and its 3D attachments disappear. Other avatars and HUDs remain visible.
  `sit-ml-body-on|sit-ml-body-off` and
  `sit-ml-reclined-body-on|sit-ml-reclined-body-off` exercise the same rules while
  seated (the same frame numbers as the seated camera scenarios). Returning to
  third person restores every own part. Preferences › Camera › Mouselook has
  the persisted switch, initially off; `AURORA_DEMO_OPTIONS=camera/2` opens it.
  Capture 605/610/615/620/625 during entry: the head and head attachments stay
  visible through the approach and disappear only when the camera reaches
  first person. `ground-ml-body-on|ground-ml-body-off` sit on the ground at
  520 with a folded-leg/lowered-pelvis fixture (instead of the demo asset's idle
  fallback), use the same first-person sequence, and stand at 760; capture 650/800
  to check posed eye height and standing restoration. The separate persisted
  **Attach the camera between the eyes** preference defaults off. With it on,
  every mouselook pose follows the current eye midpoint's position while the
  viewing direction remains controlled by the mouse. `ml-eyes-on-body-on` /
  `ml-eyes-off-body-on` animate the head for comparison (650/680); prefix
  `ground-` to exercise the same setting on the ground.
  Right-button scenarios also take physical cursor coordinates `(x,y)`:
  `right-click,x,y` moves slightly on both axes; `right-jitter,x,y` repeats
  small back-and-forth movements for 60 frames. Both must keep the camera
  still and open the context menu on release at frame 661. `right-drag,x,y`
  and `right-slow,x,y` move continuously with the button held (6 pixels and
  1 pixel per frame respectively): orbit starts after the 10-point displacement
  threshold and no menu opens on release. Capture at frames 590 and 700;
  `demo camera` logs the orbit and menu states. Combine with
  `AURORA_DEMO_PIE=1` to check the pie menu.
  `scripted`: activate a following script camera at frame 300, walk at
  380–460, activate a second source with locked eye and focus at 500 and
  walk at 580–660. Clear that source at 700 (the first resumes), then clear
  the first at 850 (normal rear view). Capture at 480, 640, 800 and 920 to
  compare the follow, lock, priority restoration and final clear stages.
  `scripted-sit`: the turning seat scenario, with its root's follow camera
  activated at frame 320 and a locked camera from its linked prim at 500.
  Standing at 700 sends only an avatar parent update, without a camera-clear
  message: both seat sources must be deactivated locally, and the camera
  must return to third person. Capture at 640 (locked) and 760 (released).
- **`AURORA_DEMO_TP`**: teleport after the fade of the initial loading
  (frame 240 at the earliest). `1` or `"x,y,z"`: immediate local arrival,
  without a loading screen; `remote` simulates the progress between regions.
- **`AURORA_TP_SCREEN`**: `legacy` brings back the teleport screen of
  before 2026-10-17 (0.8 s at least after the arrival, then a bar easing at
  2.6 per second up to 98.5 %), `alt` uses it every other teleport, starting
  with the first. The quick screen leaves once the terrain is there and
  0.4 s have passed, its bar finishing in 0.15 s
  (`app/teleport_screen.rs`). With `AURORA_PROFILE=1` the teleport
  timeline ends `arrived (teleport screen hidden)`, or `…, old screen)`.
- **`AURORA_DEMO_TP_AWAY=1`**: at frame 760, our avatar arrives in a region
  25 km from the demo one, as after a teleport; at frame 960, it comes back
  there, before the demo region's simulator has closed it. The objects of
  the demo region are set aside on departure (`world/parked.rs`) and put
  back as they were on return; the log gives `objects parked`, then
  `objects back`. With `AURORA_FPS_LIMIT=60` and
  `AURORA_CAPTURE_FRAMES=700,900,1500`: the plaza, then our avatar alone
  above the water, then the same plaza (objects, floating texts, other
  avatar, particle fountain). In `perf summary`, `objects=` goes from 57 to
  1 then back to 57, with the same `visible=` and `draws=` as before.
  The arrival away also forgets the demo region's coarse locations, and
  the list it still sends at frame 800 waits with its objects (#1000;
  log: `still sends its coarse locations`): with
  `AURORA_DEMO_CONV=aurora:friend-away` and `AURORA_DEMO_CONV_MENU=im`,
  Nova's Zoom is enabled at frame 740, disabled at frame 900 (Teleport To
  stays, through her map rights), and enabled again at frame 1000.
- **`AURORA_DEMO_AGENT_JITTER=<mm>`**: our avatar receives five times a
  second (at 60 fps) a position correction of at most this size, like those
  a simulator sends to an avatar standing at altitude. With
  `AURORA_DEMO_CAMERA=close,x,y` on a rezzed object, the frame stays
  identical to the pixel; on our avatar, the camera follows it and all the
  nearby scenery jumps. Combine with `AURORA_DEMO_ALTITUDE` and
  `AURORA_STILL_CHECK=1`.
- **`AURORA_DEMO_KEY=down|up|left|right`**: holds an arrow from frame 235
  (movement, body orientation). `:pulse` (`up:pulse`): held 2 s, released
  2 s, over and over (by the clock: the same at any frame rate).
- **`AURORA_DEMO_LINK=<ping ms>[,jitter=<ms>][,stoplag=<ms>][,aurora|firestorm][,vehicle|crash][,trace][,native-flight]`**:
  instead of moving our avatar at once, the demo sends the keys to a
  stand-in simulator after half the ping; it moves its own copy of the
  avatar with the demo's rules and its reports (terse updates of our
  avatar, its walk / run / fly / stand animations) come back after the
  other half, stamped with their arrival like the network's
  (`demo_link.rs`). The viewer then runs the grid's path: extrapolation,
  smoothing and, with Aurora's movement (Preferences › Aurora › Movement,
  "Instant"), the prediction of `own_motion.rs`. `jitter` delays each
  message by up to that much more (order kept); `stoplag` keeps the
  stand-in's walk signal that long once it has stopped, as a busy
  simulator is late to say it (with `trace`, the drawn animations of each
  frame show whether the walk comes back after a stop); `aurora` / `firestorm`
  force the movement for the run without touching the saved choice;
  without them, the choice of Preferences applies and can be switched
  while walking. `native-flight` opts into a 16 m/s flight model with a
  0.5 s velocity ramp, coasting after release and full speed despite NUDGE.
  It makes avatar-navigation braking tests exercise native flight inertia;
  ground movement and the default demo physics keep their existing behavior.
  The log says, for each press and release of a walking
  key, `demo link: the drawn avatar moved N ms after the press` and
  `… stopped N ms after the release, D m on (P m travelled)`, and
  `own motion learned: …` (the speeds, ramps and delay the prediction
  learned, as on a grid). Measured on 2026-10-16 with `200` and
  `AURORA_DEMO_KEY=up:pulse`, `AURORA_FPS_LIMIT=60`: Firestorm's movement
  starts 250 to 317 ms after the press and stops 500 ms and 0.9 m after the
  release; Aurora's starts within 50 ms (the first frame) and eases to a
  stop over about 0.15 s, 0.2 m on, without coming back (`jitter=40`: the
  same). `trace` writes one `demo link trace` line per frame (keys, drawn
  position, simulator's position, lead, correction left, camera and its
  focus, drawn animations): with `AURORA_DEMO_KEY=up:pulse`, the speed frame by frame
  around each release must fall without a step and never rise again
  (measured 2026-10-16, 60 and 500 fps: at most 0.5 m/s less from a frame
  of 16 ms to the next, no rise, no pull-back; Firestorm's: up to 1.16 m/s
  per frame). `vehicle`: at 2 s a car appears, at 3 s our
  avatar sits on it, from 4 s to 16 s the car drives back and forth at
  4 m/s with a report every quarter of a second, at 17 s the avatar stands
  up beside it. With `AURORA_DEMO_KEY=up`, the keys held while seated must
  not move the avatar (it stays on the car, extrapolated and smoothed like
  any object), and standing up must start from the simulator's position,
  then walk at once. `crash` (implies `vehicle`): at 3.25 s the car takes
  the camera like a racing script (rigid behind it, no lag), drives east
  at 6 m/s, hits a wall at 7 s and another at 11 s on the way back: each
  impact is a few abrupt reports (thrown back, turned 17° and rolled 7°,
  spinning) that the next ones undo. The car and the camera following it
  must swing through the impact without a jolt: the root an avatar sits
  on is damped every frame (issue #1250). With `trace` (camera and focus
  per frame), measured on 2026-10-28 at `AURORA_FPS_LIMIT=60`: during an
  impact the view sweeps 34–38° (49° before the damping), peaking at
  137–163°/s (189°/s), and the camera's velocity changes by at most
  21 m/s from a frame to the next (35 m/s). Plain `AURORA_DEMO` (no link) has nothing to predict:
  the setting has no effect there. Any prim of the demo can be sat on
  (right-click › Sit, or `/siton <key>` in the chat bar) and stood up from
  (Stand up), with or without the link: our avatar sits on its top and
  stands up beside it; through the link both answers come back one round
  trip later, and the stand-in stops reporting while seated. With
  `AURORA_DEMO_ACTIONS=1` a row of cubes stands in front of the avatar
  (the Sit chair keeps its own answer without the link).
- **`AURORA_DEMO_SIT=n`**: clicks at frames 240, 300, 360…
- **`AURORA_DEMO_SIT_TP=local|region`**: at frame 300 our avatar sits on
  the nearest prim of the plaza, at frame 420 it teleports from the seat,
  7 m south-west (`local`, in the region) or by the demo's remote teleport
  (`region`, 3 s of progress). The demo's simulator unseats it and stops
  the sit animation but sends no update with its new parent, as a
  simulator whose updates after a teleport are terse ones: the viewer gets
  off by itself (`get_off_for_teleport`, LLAgent::teleportCore). With
  `AURORA_FPS_LIMIT=60`, capture frames 380 (seated) and 700 (arrived):
  standing at the destination. The log gives `demo sit tp: before the
  teleport, seated true`, `teleport: our avatar gets off its seat`, then
  `demo sit tp: arrived, seated false, parent 0, seat animation false`
  (before the fix: `parent 150, seat animation true`, the avatar drawn
  seated far out of the region).
- **`AURORA_DEMO_MMO="x,y[,1|2|3]"`**: left press on the avatar then right
  button held (steering with the mouse); `1`: right arrow instead (the
  avatar slides right, its yaw unchanged), `2`: double right click
  (running), `3`: down arrow instead (the avatar backs up without turning:
  the `backing up from` and `after walking` yaws match, #1259). Additional
  comma-separated flags:
  `world` enables the world mouse-walk preference for this demo; `world-off`
  disables it. Aim at land or a noninteractive object to compare both.
  `left-first` releases the left button before the right; by default the
  right is released first. `right-before-left` presses the right button first.
  `quick` queues both buttons and both releases behind the deferred left
  pick in the same frame. `menu-after` adds an ordinary right click after
  both releases. The log reports walking, release consumption and
  menu state: a walked chord must never open a context menu, and an ordinary
  subsequent right click must still work. Interactive objects, HUDs, media,
  build tools, modified clicks and ground double-click actions retain priority.
- **`AURORA_DEMO_MENU_SEARCH=<query>`**: the wolf menu of the top islands
  (the default bars; the click misses the wolf of the Legacy bars) is
  clicked at frames 540–541 and `query` is typed into its search field at
  560 (the field takes the keyboard when the menu opens). Capture frame
  620: the matching entries of every sub-menu under their path heading
  (`home`: Go home under Me, Set Home to Here under World), or the "No
  entry matches" line. A broad query (`e`) scrolls under the field, and a
  path met again (Me after Me › Avatar health) gets a divider, not its
  heading a second time.
- **`AURORA_DEMO_DOUBLECLICK=walk|run|tp[,x,y]`**: the double-click action
  of the session (the saved one is left alone), and a double click of the
  left button at pixel (x, y) at frames 700 to 703 (ground ahead and to the
  left of the avatar by default), through the same way in as the mouse.
  With `AURORA_FPS_LIMIT=60`: capture frame 712 to see the destination
  mark (a teal dot) and the body turning toward it, 780 for the arrival or
  the teleport. The log says `demo double click: …` and `demo walk ended
  (done|cancelled): …`. `menu` opens the wolf menu at frame 540, Me at 560
  and its "Double-click on the ground" entry at 580 (`menu,y`: another
  height in points): capture frame 620.
- **`AURORA_DEMO_RCLICK`**: right click at (x, y) at frame 225, or with
  `tag` on the name plate of the nearest other avatar at frame 600.
- **`AURORA_DEMO_PIE`**: `1` forces the pie menu, `0` forces the list, only
  in demo and without changing the saved choice. Combine with
  `AURORA_DEMO_RCLICK="x,y"`, `tag` or `AURORA_DEMO_HUDS=menu` to compare
  the same targets. For the human test of the selector, remove this
  variable: Preferences › Interface › Layout › Right-click menu; switch
  between List menu and Pie menu (radial), then restart to check the choice.
- **`AURORA_DEMO_TEXT_MENU=1`**: the chat bar takes the focus at frame 700,
  gets a line typed and selected at frame 703, and the text fields'
  right-click menu opens over it at frame 706 (capture at frame 780 with
  `AURORA_FPS_LIMIT=60`). Cut, Copy, Delete and Select All are enabled,
  Paste only with text in the clipboard. By hand: right click in any text
  field; a right click inside the selection keeps it, elsewhere it moves
  the cursor; a password field never offers Cut or Copy, a read-only one
  only Copy and Select All; the script editor and the notecards keep their
  own menu.
- **`AURORA_DEMO_POINTER="x,y[,l|r|s|h][;x,y…]"`**: moves the interface
  pointer to these pixels of the window from frame 300, one point every
  60 frames (hover states, submenus); `,l` makes a left click, `,r` a right
  click on the interface at that spot. In the pie menu, the left click
  enters a submenu and the right click goes up one level. `,s` leaves the
  pointer still at that spot (it is only brought back there if it has moved
    away): this is what is needed to capture a tooltip, which egui only shows
    under a pointer at rest. `,h` holds the left button for 57 frames; use
    `AURORA_FPS_LIMIT=60` to exercise the 0.5-second hold that opens a
    person's contact settings in Contacts and People. Compare with `,l`:
    a short click selects the row and keeps the list visible.
- **`AURORA_DEMO_HOVER=1`**: from frame 300, the cursor sweeps the window
  for 120 frames, stays still for 120 frames, and so on (search for the
  object under the cursor and answers kept from one frame to the next,
  `scene/hover.rs`). With `"x,y"`, the cursor stays on that pixel of the
  window from frame 300 (on an avatar of the crowd, on the ground next to
  it). Read `hover` and the `hover summary` lines in the profile; combine
  with `AURORA_HOVER_CHECK=1`, `AURORA_DEMO_ACTIONS=1` or a load scene.
- **`AURORA_DEMO_CLICKS="<ms>[,x,y|sweep|rigged][,left|alt|menu|build|focus]"`**:
  one click on the world every `ms` milliseconds (250 if the value is not a
  number) once the world is displayed, button held 60 ms, through the same
  path as a mouse button (`app/click_pick.rs`), without ever taking the real
  cursor (Alt + click, click on our avatar). A line
  `demo clicks: the click before is not over` reports a click whose release
  was not processed before the next one. At pixel (x, y) of the window (a
  quarter of the width, near the bottom, by default: the ground), or with
  `sweep` at a new spot of the window at each click, or with `rigged` on the
  rigged meshes on screen, in turn (in the middle of their posed bounds).
  `left` (default): left click; `alt`: Alt + click (camera); `menu`: right
  click; `build` and `focus`: click with the Edit tool or the Focus tool of
  the build tools. Used to measure what a click costs the frame in a loaded
  scene (`AURORA_DEMO_CROWD=250`, `AURORA_DEMO_ANIMESH=1`,
  `AURORA_MAX_AVATARS=300`, `AURORA_AA=6`: the GPU is the limit there): with
  `AURORA_PROFILE=1`, read `between` in `max:` and `off_cpu:`, and the
  `click pick:` lines (one per click: frames and milliseconds before the
  answer). Combine with `AURORA_CLICK_PICK=wait` or `check`.
  - Clicks on what is rigged: `AURORA_DEMO_ANIMESH=1`,
    `AURORA_DEMO_CROWD=4`, `AURORA_DEMO_POS="131,126.6"` and
    `AURORA_DEMO_CLICKS=150,rigged,menu` (or `left`, `build`) put on screen
    a rezzed animesh (objects 9200 and 9202), an animesh worn by another
    avatar (9203), the rigged mesh of the crowd and a rigged mesh worn by
    our avatar (9210, added by this mode), all made sensitive to touch.
    Each click writes a `demo drawn pick` line: the object aimed at
    (`aimed at … : OK` or `MISSED`, the latter when something else hides
    it), the object picked, its face, and `inside its posed bounds`
    (`OUTSIDE` would be a defect: the point read on the GPU is not within
    the bounds the CPU knows for the object). Then come `demo menu: …`
    (menu opened) or `demo click action: Touch, object=…`. With
    `3000,rigged,menu` and `AURORA_CAPTURE_FRAMES=1100,1460`, the
    screenshots show the menu of an object worn by another avatar then that
    of an object worn by us, the object surrounded by its outline.
- **`AURORA_DEMO_POINTING`**: at frame 600, select a cube in front of our
  avatar (`1`), or receive another avatar's target (`remote`). `release`
  closes Build at 840; `private` enables selection privacy then;
  `remote-release` receives a clear then. Capture at 760 and 940 to check the
  arm pointing and its half-second release. `aim` adds a synthetic gun pose
  to exercise the additive torso targeting motion. No grid assets are used.
  The fixture disables the demo account's AO and Movelock and closes the
  inventory so the gesture is visible.
  `AURORA_DEMO_CLASSIC=1` supplies the remote avatar's appearance. Keep
  that avatar rendered fully instead of as a complexity silhouette.
- **`AURORA_DEMO_LOOKAT=1`**: gaze tracking enabled (in memory only) and
  gaze of a remote avatar; the cursor is held on the left of the window,
  moving by a pixel. **`AURORA_DEMO_LOOKAT=own`**: the same with
  Preferences › Privacy › "Show look-at targets" on, our own target and the
  lines shown (in memory only), and the real cursor: our crosshair stands
  1 m from our head, toward the cursor (Firestorm's free look, not on what the
  cursor points at). `own,x,y` holds the cursor at (x, y), fractions of the
  window: with `own,0.15,0.45`, `own,0.85,0.45`, `own,0.5,0.1` and a capture
  at frame 600, our crosshair stands left, right, then above our head, or
  on what the cursor is over (the head looks at that point).
  `own,x,y,side` also turns the camera beside our head from frame 300,
  aimed at a point in front of the face, which we then look at: the line
  leaves from between the eyes (capture at frame 420).
  **`AURORA_DEMO_LOOKAT=ahead`**: from frame 300 the camera in front of our
  face and our eyes on a point far to our left (a focus), then from frame
  500 Privacy › "Send no target at all, not even to me" on (in memory): head
  and eyes come back straight ahead (captures at frames 480 and 640),
  instead of the eyes staying on the last point.
  **`AURORA_DEMO_LOOKAT=targets`**: `own`, and from frame 240 every other avatar looking
  somewhere new every 120 frames (focus and select points, the next avatar's
  eyes, free look, mouselook: one colour per type). With
  `AURORA_DEMO_CROWD=12`, `AURORA_DEMO_POS=129,134`,
  `AURORA_DEMO_CAMERA=close,1180,380,35`, `AURORA_FPS_LIMIT=60` and a capture
  at frame 860: crosshairs, names and lines among the crowd, hidden by the
  avatars in front of them.
  **`AURORA_DEMO_LOOKAT=leave`** (issue #482): `own` with the real cursor
  (`leave,x,y` holds it like `own,x,y`); at frame 230 Violet Wolf looks at
  us (his mark and name on our head), at frame 800 he logs off (his avatar
  is killed). With `leave,0.15,0.3`, `AURORA_FPS_LIMIT=60` and
  `AURORA_CAPTURE_FRAMES=760,900`: his mark on our head, then gone with
  him. The log gives `demo lookat leave: frame 800, remote look-ats 0`.
  Our own mark follows the cursor, and goes back ahead of us when the
  cursor rests: after 1 s on a point under the cursor, 2 s over the sky or
  with the Legacy aim (the hover and free-look timeouts of the attention
  table, as in the Legacy Viewer).
- **`AURORA_DEMO_KEYBOARD=system|wasd|zqsd|qwertz|fallback`**: fresh default
  values of the movement keys and of the letter shortcuts with the system's
  layout (Windows, or the XKB keymap of the Wayland compositor or the X
  server on Linux), a simulated US QWERTY / French AZERTY / German QWERTZ,
  or a failed detection (US QWERTY places); combine with
  `AURORA_DEMO_OPTIONS=8` to examine the shortcuts. With `zqsd`, Mouselook
  shows M, World map Ctrl+M and Snapshot to disk Ctrl+Ù (the AZERTY keys
  they are bound to); with `qwertz`, Snapshot to disk shows Ctrl+Ö. Saved
  bindings keep their keys, except the one-time move of the former US QWERTY
  defaults (settings version 12).
- **`AURORA_DEMO_CHATCMD="calc 2+2;rolld 2 20"`**: lines typed in the chat
  bar at frame 240, separated by `;` (chat bar commands: `calc`, `rolld`,
  `gtp`…).

## Objects and building

| Variable | Effect |
|---|---|
| `AURORA_DEMO_ACTIONS=<mode>` | Cursors, object clicks, buying and paying |
| `AURORA_DEMO_HOVER_TIPS=<form>[:all][:noland]` | World tooltips over objects, avatars and land |
| `AURORA_DEMO_WORLD_CLICKS=<mode>` | Camera transparency, name-plate clicks, scripted attachments and the left button's script clicks |
| `AURORA_DEMO_DIALOG=<mode>` | llDialog menu, every scripted object's window |
| `AURORA_DEMO_BUILD="<script>"` | Script of the build tools |
| `AURORA_DEMO_BEACONS=1` | Object beacons and World overlays |
| `AURORA_DEMO_REPLACE_PRIM=1\|script` | Replacement board, script-only case |
| `AURORA_DEMO_BUILD_FRAME=<n>` | Start frame of the build script |
| `AURORA_DEMO_BUILD_FOCUS=off\|on` | Focus and camera following while editing |
| `AURORA_DEMO_BUILD_HANDLES=<percent>` | Size of the edit handles |
| `AURORA_DEMO_BUILD_EDGE=<edge>` | Move to the edge, hold then release |
| `AURORA_DEMO_BUILD_ECHO=<ms>` | The demo answers transform edits a round trip late |
| `AURORA_DEMO_BUILD_TAB=<tab>` | Tab of the build window; `features:light-picker` opens the projector texture picker |
| `AURORA_DEMO_BUILD_SIZE=<width>,<height>` | Initial dimensions of the construction window |
| `AURORA_DEMO_BUILD_STYLE=aurora\|legacy` | Construction window presentation for this demo run |
| `AURORA_DEMO_PARTICLE_EDITOR=1\|fountain\|inject` | Build › Edit particles on the object selected by `AURORA_DEMO_BUILD` |
| `AURORA_DEMO_COLOR_PICKER=<mode>` | Colour picker and gestures offline |
| `AURORA_DEMO_SCRIPTS=<mode>` | Editing and saving scripts offline |
| `AURORA_DEMO_LSL_BRIDGE=1` | Messages of the Firestorm LSL bridge (hidden) |
| `AURORA_DEMO_MOVELOCK=<mode>` | Bridge lifecycle/lock and native sitting choice; see details below |
| `AURORA_DEMO_BRIDGE_QUERY=radar\|scripts\|object\|error` | Bridge altitude correction, basic script and extended object-information reports |
| `AURORA_DEMO_MOVEMENT=surface\|underwater\|breaststroke\|backstroke\|crawl\|independent\|float-low\|float-high\|off\|run-fall\|run-ao\|run-drop\|passage-blocked\|passage-reached\|passage-pending` | Movement options, local swimming poses and synthetic passage feedback |
| `AURORA_DEMO_FLEXIBLE=1` | Flexible prims: gravity, tension, wind, force and moving parent |
| `AURORA_DEMO_TEXREFRESH=1\|auto` | "Refresh textures" on a panel whose cached texture is stale |
| `AURORA_DEMO_SCULPT_ZERO=1` | Subtype-zero sculpt classification beside torus and sphere controls |
| `AURORA_DEMO_SCULPT_MISSING=1\|late` | A sculpt whose map is missing: the sphere placeholder, then its torus with `late` |
| `AURORA_DEMO_TEXTURE_MISSING=1` | Boxes whose texture is missing: the Legacy Viewer's grey, never the rez grid for ever |
| `AURORA_DEMO_BOM_REZZED=1\|pbr` | Ground BOM placeholders, or PBR BOM maps on a ground box and an attachment |
| `AURORA_DEMO_OBJECT_MENU=<mode>` | Object actions, scene menu customization/notecards, object profile and pathfinding readers |

**Beacon details.** `AURORA_DEMO_BEACONS=1` adds seven labelled sources, including an invisible scripted cube. `lines`, `highlights` and `off` isolate the two render modes; `hidden-particles` keeps the source marker while hiding emitted particles. `parcels` frames the terrain boundaries and ownership colours. `owned` opens My land with an offline parcel and group contributions. Check each source switch, Scripted versus Touch, width, both master switches off, and closing/reopening the floater without losing the active filters. The grid check compares real sound/particle/media sources, linked children, attachments, neighbouring parcels and the personal holdings list with the Legacy Viewer.


### Details

`AURORA_DEMO_PARTICLE_EDITOR` needs an object of ours selected by the build
script, for example `AURORA_DEMO_BUILD=move,object:151`. At frame 700 the
particle editor opens on it: `1` with the window's initial values (white
particles exploding), `fountain` with a glowing violet-to-turquoise cone that
falls back, and `inject` also presses Inject at frame 760 (the window shows
"put into the object"; the demo inventory gets "New Particle Script" in
#Aurora). Look for the preview on the prim, the greyed fields (end colour and
scale follow the interpolation flags, angles the pattern) and, with `1`, the
prim's own particles hidden while the editor is open.

`AURORA_DEMO_BUILD=physics,object:99621` selects an encoded ring mesh with a
separate triangular collision surface. `physics-analysed` uses two convex
decomposition hulls; `physics-convex` shows the single base hull; `physics-none`
and `physics-off` leave no collision overlay. `physics-cycle` switches the eye
off, on with Convex hull, then back to Prim. Use `object:99620` for the torus
prim instead. The Features section opens to physics. Compare the translucent
green surfaces and dark triangle edges, then toggle the eye repeatedly, change
shape type, select a linked child, move/rotate/scale it and close construction.
Only the selected collision shape should appear; materials, picking and shadows
must stay unchanged. Human grid validation covers uploaded analysed/unanalysed
meshes, linked parts, None, flexible prims, HUDs and re-opening the inspector.

`AURORA_DEMO_BUILD_HANDLES` sets the construction setting "Edit handles ›
Size" (50 to 300, in percent) when the script opens the build tools, for
example `AURORA_DEMO_BUILD=move,object:151` with `150`. Compare the arrows,
plane squares, rotation rings (`rotate`) and stretch handles (`stretch`)
with a 100 % capture: everything grows by the same factor, strokes
included, and a handle can be grabbed anywhere on its larger shape.

`AURORA_DEMO_BUILD_STYLE=legacy` selects the Legacy tool strip and five tabs,
independently of the saved preference. Use `AURORA_DEMO_BUILD=move,object:118`
with `AURORA_DEMO_BUILD_TAB=general|object|features|texture:bp|texture:pbr|contents`.
Compare with `floater_tools.xml` and `panel_fs_tools_texture.xml`: the compact tool
block ends with linked-part navigation, Link/Unlink and the ruler on one row;
counters sit directly above the five tabs. General places sale before permissions.
Object has flags and vertical XYZ/C/P/p blocks on the left, shape/mesh data on the
right. Features exposes its flexible/physics columns and light/probe controls
without folding cards. Texture always exposes PBR, Blinn-Phong and Media as
three adjacent buttons, including with no editable faces. Blinn-Phong shows its
three maps vertically with tints beside them and surface parameters on the right.
PBR shows material/base color, normal/emissive and ORM rows, tint swatches and live
inline factors. Channel tabs sit above the Scale/Offset/Rotation columns; repeats,
sync, planar alignment, hide-water and mapping form the footer. Check mode and
channel switching, all map pickers and tint transactions, alpha/material factors,
vector copy/paste, linked selection, ruler settings and the bottom collapse strip.
At 330 by 620 the property panel scrolls while the tool block and tabs stay
in place.
Switch Preferences > Aurora > Build tools between Aurora and Legacy while
editing; the selection, properties and custom Aurora arrangement must survive.
The bulk Aurora/Legacy choices include this preference, which persists on restart.
Legacy keeps Aurora's theme and native mesh inspector while following the
reference control arrangement. Human grid validation covers
actual property/transform edits, permissions, linking and object contents.

`AURORA_DEMO_BUILD_SIZE` sets the construction window's initial dimensions
once for reproducible captures; normal sessions restore saved geometry. The chosen
presentation stays the same at every width. Aurora starts at 350 by 820 and Legacy
at 360 by 780; Aurora's visible icons and expanded transform area
can raise its minimum height. Append
`+settings`, `+organize` or `+folded` to `AURORA_DEMO_BUILD_TAB=general` to inspect
the side settings, hidden-item storage/notecard buttons or folded numeric values.
The visible rail icons set the minimum editor height: attempt to shrink the
window, then hide/restore icons and repeat. Every remaining icon must fit without
a rail scrollbar. Check the settings gear remains square with values open/closed.
The old `+grid` suffix now opens the same side settings with inline ruler controls.
Human validation: move the main window to both sides; settings follow its free
side and bottom. Organize options without changing their checkbox states, move
both toolbar rows and rail icons into hidden storage and restore them. Export a
named notecard, change the layout/ruler, load that inventory notecard and confirm
restoration without changing object transforms. Check a foreign or malformed
notecard reports an error without changing any settings. Construction preferences
and layout save automatically in the local global settings file, shared across
accounts. Notecard export is explicit. Grid login and inventory roundtrip are
performed by the human; demo exports use synthetic inventory items.

For the material layout, use `AURORA_DEMO_BUILD=move,object:125` and
`AURORA_DEMO_BUILD_TAB=texture:bp+folded` at 350 by 820: three square previews
are in one row, followed by tint/surface, relief/reflection and UV controls.
For PBR, add `AURORA_DEMO_PBR_OVERRIDE=1`, select `move,object:150` and use
`texture:pbr+folded`: a square rendered material preview and its four maps share
the BP preview size in a three-column grid,
then tint/surface, light response and per-channel UV controls. Scroll the main
section to reach lower controls; the shared footer stays in place. Check each
preview opens the matching picker, colors keep their live preview/cancel,
alpha cutoffs appear in Mask mode, and per-channel UV values remain independent.
Object 125 also checks the built-in white diffuse image. On the grid, assign
White, Transparent and Default in turn: the first is white, the second shows a
checkerboard, and Default displays its texture even when already resident or
loaded from cache. Check PBR material factors/texture overrides update its preview.
On object 125, the specular tint must show a lock; hover the disabled swatch
to see the missing-map tooltip. Switch to PBR to see both tints locked,
with the missing-material tooltip on hover. No explanation occupies a separate row.
Applying a specular map or PBR material must restore the corresponding colour
controls. Check both languages; PBR factors remain editable without colour maps.

For selected-object display switching, keep `AURORA_DEMO_PBR_OVERRIDE=1` and
`AURORA_DEMO_BUILD=move,object:151`. Compare `texture:bp+folded` with
`texture:pbr+folded`: the selected slab has its plain classic texture in BP
and tiled PBR texture with its UV/tint override in PBR; the other slab retains
its PBR rendering. In the native viewer, click BP, PBR and
Media repeatedly, select another object/linked part/face, then leave Materials
or close/fold the editor. Each object must retain its last mode after
deselection, independently of the next object. Use
`AURORA_DEMO_BUILD=material-deselect,object:151` for an automatic native capture
after closing/deselection: BP must remain plain and PBR must remain tiled.
Reselect an object and choose PBR to restore its material, including every
material/map/UV override. Leave the region by crossing or teleporting, then
return: all local modes must be reset, including for cached objects. A teleport
within the same region must keep them. Disconnect/reconnect must also reset
them. No simulator material update is sent by changing tabs; BP texture edits
still follow their normal edit commands.

`AURORA_DEMO_SCULPT_ZERO=1` places three objects east of the start position
(x=134, y=123/125/127). The grey object has no sculpt block and stays a torus;
the violet subtype-zero sculpt and the teal sphere sculpt both use a
synthetic 64×64 lossless sphere map and must look spherical. All three share
the same torus path/profile. The violet object becoming a torus reproduces
the classification failure; subtype zero must also retain its unstitched
edges rather than silently using sphere stitching. The map is generated
offline in the demo cache; no grid asset is used.

`AURORA_DEMO_SCULPT_MISSING=1` places a gold sculpt east of the start
position (x=134, y=130), north of the `AURORA_DEMO_SCULPT_ZERO` objects, at
frame 300. Its map is in no cache and the asset server answers 404: the
sculpt shows the Legacy Viewer's sphere placeholder (radius 0.3 of its size)
instead of nothing. With `late`, the map turns up at frame 900 (written to
the disk cache and read again, as a retry finding it on the server): the
torus it describes replaces the sphere. A map still loading draws nothing,
as in the Legacy Viewer. Capture with `AURORA_FPS_LIMIT=60` around frames
700 and 1100.

`AURORA_DEMO_TEXTURE_MISSING=1` places two boxes east of the start position
(x=134, y=133 and 135.5) at frame 300, a white one and a dark, mostly
transparent one. Their texture is in no cache and the asset CDN answers 403
(a missing asset). With the rez effect on (Preferences › Aurora), they show
the grid only while the texture is asked for, then the plain grey the Legacy
Viewer draws for a missing texture (the dark box: a faint dark veil). Before
the fix they stayed the grid for as long as they were in view. Capture with
`AURORA_FPS_LIMIT=60` around frame 700.

`AURORA_DEMO_BOM_REZZED=1` places a box ahead of the start view (x=135,
y=129.5) whose faces carry the bake-on-mesh placeholders of the head, the upper
body and the lower body, like a BOM body part rezzed out. The demo stands in
for the grid's placeholder textures with labelled diagonal stripes: orange (head), teal
(upper body) and violet (lower body). Not worn, the box shows them, as the
Legacy Viewer does; before the fix its faces were white. On the grid the
placeholders are the real "baked" channel textures.

`AURORA_DEMO_BOM_REZZED=pbr` adds a material with BOM in all four channels,
on the ground box and a chest attachment (`81141`). Both demo avatars supply
skin, normal, ORM and emissive bakes. Use
`AURORA_DEMO_BUILD=pbr-bom,object:81141,0` to frame the attachment and open
its map picker; the final number selects colour (0), normal (1), ORM (2) or
emissive (3). The picker, swatches and material sphere show the coloured
`BAKED_*` reference images while the worn object uses the avatar's bakes.
`AURORA_DEMO_BUILD=bp-bom,object:81141` shows the same reference-image preview
for a Blinn-Phong diffuse map. The ground box keeps the placeholders.
Check both `AURORA_DEMO_BUILD_STYLE=aurora` and `legacy`.

`AURORA_DEMO_PROJECTOR=1`, `AURORA_DEMO_BUILD=move,object:9712` and
`AURORA_DEMO_BUILD_TAB=features:light-picker` select a light and open its
projector texture picker. Choose None, then OK: the light stays on with the
same colour, intensity, range and falloff, and its projector controls become
disabled. Cancel instead restores its texture and projector settings. Use
`features` without the suffix to inspect the light before or after editing.

`AURORA_DEMO_WORLD_CLICKS=camera-zero|camera-te|camera-glass|plate-hidden|plate-wall|worn-prim|worn-mesh|worn-own|worn-own-mesh[-drag]|worn-other|worn-other-link|worn-other-covered|worn-other-under|worn-other-mesh|worn-other-hidden|worn-other-hidden-under|worn-other-hidden-mesh|worn-other-hidden-mesh-under|worn-other-hidden-inside|worn-other-moving|mouselook-gun|mouselook-touch|ground|photo-touch[-ctrl]|photo-hud[-ctrl]`:
install the fixture at frame 600, issue a real left press at 700 and release
at 701, with the asynchronous GPU pick and input queue. Capture at frame 780
or later. `worn-other-moving` makes the scripted prim a 10 cm clicker and
moves its wearer 8 cm a frame from 701 to 705, while the GPU answer of the
press is on its way, as an animated avatar does: the press must still touch
it (`demo touch_start: prim=975`). Add `-held` to a `worn-*` mode to hold the
touch and slide the cursor over the attachment from 710 to 760, released at
761 (capture at 840): every `demo touch / grab update` line must give a new
`st` (none repeats the `touch_start` one), which a script reading the touch
position sees. `photo-touch[-ctrl]` and `photo-hud[-ctrl]` open the photo viewfinder:
Ctrl + left click touches the scripted cube or the HUD without moving the AF
point, then releases the touch even if Ctrl is released before the GPU pick
returns. The plain cube click moves the AF point instead; a visible HUD also
keeps its plain click. `demo photo press` logs restored keyboard modifiers;
`demo photo interaction` logs the AF point, dialog and released button.
`avatar-keep-camera` starts from an off-centre side view and enables
Preferences › Camera › Behavior › “Click my avatar: keep camera” for the run:
a plain body click must preserve position, target, lens and tracked object.
`avatar-return` disables the option and must return to avatar follow.
`avatar-drag` / `avatar-tag-drag` hold a real press on our body / own plate
at frame 700, drag horizontally at 710–760 and release at 761. Add
`-keep-camera` to retain the off-centre focus. Capture at 690, 780 and 840:
the press keeps the view with the option enabled, then both the view and
the avatar turn; the log records agent and drawn-body yaw before/after.
The focus object, lens and distance must remain stable during the kept-focus
drag. Compare `avatar-keep-camera` for a plain click without motion.
`avatar-alt-keep-camera` must still retarget with Alt held; `avatar-other-keep-camera`
must leave the view unchanged on another resident. The `-keep-camera` suffix
also works with `worn-own` (scripted touch) and `worn-own-passive` (no script):
both keep the camera, while only the scripted attachment receives a touch.
`demo world click camera` logs the view immediately before and after the press;
capture frames 690 and 780 to compare. `camera-zero` puts a zero-alpha Blend texture before the Buy cube;
`camera-te` uses an invisible scripted TE face. Alt-focus must reach the cube
behind both; `camera-glass` instead reaches the translucent solid pane.
`camera-faint` uses texture alpha 1/255: it must still take the focus, since
only alpha-zero texels pass through world Blend objects (#1574).
`camera-mask` uses a fully transparent Mask texel and also reaches the cube.
Add `-ctrl` to any `camera-*` mode to press Ctrl + Alt together. Repeat with
`AURORA_DEMO_DEBUG=alpha`: all four modes must focus the highlighted panel
(976) instead of the cube (971), including Mask holes and invisible TE faces;
`demo world click result` logs the focus point and target. `camera-occluded`
adds an opaque wall (977) in front, which must still take the focus with or
without the overlay. Rigged faces participate only when
their transparency overlay is displayed (`alpha_rigged` too).
`plate-hidden` opens the profile of another resident through an invisible
Blend panel; `plate-wall` keeps the profile closed behind a solid wall.
Append `-shift` or `-ctrl` to either plate mode to select the independent
Shift action (IM) or Ctrl action (avatar menu); the plain action stays Profile.
The modifier is released at frame 701, before a delayed GPU pick may finish.
`plate-hidden-ctrl-shift` and `plate-hidden-alt` must run no plate action.
Append `-none` before `-shift` or `-ctrl` to disable that gesture: it must not
fall back to Profile. `AURORA_DEMO_OPTIONS=avatars:plates` reveals the three
independent preference rows; both new actions default to Nothing.
`worn-prim` and `worn-mesh` put a scripted attachment on another resident,
without move/modify rights; both must log `demo touch_start: prim=975` and
`demo touch_end: prim=975`, with `held=false` in the final result. The mesh
scenario installs the existing synthetic skinned mesh locally. `worn-own`
wears a scripted prim on our own chest (issue #488): the press must touch it
(`demo server: ObjectGrab 975`) instead of turning to our avatar.
`worn-own-mesh` wears the scripted rigged mesh, drawn opaque, on our own
chest and presses it on our back, off the classic body: like the Legacy
Viewer's left click, which ignores rigged meshes, the press goes to our avatar
and must log no `ObjectGrab 975`; `worn-own-mesh-drag` drags at 710–760 and
the `demo world click camera` lines show the agent yaw turning. Opaque, the
mesh gives the press its depth, inside our box whatever the pose.
`worn-other` puts that prim on another resident's chest; `worn-other-link`
adds a scriptless child above it and presses the child (`ObjectGrab 988`).
`worn-other-covered` and `worn-other-under` put the demo's rigged mesh,
without a script, over a smaller scripted prim on that chest, blended or
opaque: the press goes through the rigged mesh to the prim (`ObjectGrab
975`), as the Legacy Viewer's left click ignores rigged meshes.
`worn-other-mesh` replaces that prim with a cube mesh without a skin;
`worn-other-hidden` and `worn-other-hidden-mesh` make the prim or the mesh
invisible (every face at alpha 0, an invisible clicker), and the `-under`
variants of both put it under the opaque rigged mesh: every press must touch
it (`ObjectGrab 975`). `worn-other-hidden-inside` shrinks the invisible prim
to 10 cm, sunk into the chest well under the rigged mesh's surface, like a
clicker worn partly inside a mesh body (#1241): the press must touch it too
(`ObjectGrab 975`); before the fix it touched nothing.
`mouselook-gun` makes a script take the mouselook button
(ScriptControlChange) and enters mouselook: the press and release send only
`demo server: click bits 0x40000000` then `0x80000000`. `mouselook-touch`
enters mouselook with nothing taken and puts a touchable panel under the
crosshair: `click bits 0x10000000`, `ObjectGrab 975`, `click bits
0x20000000`. `ground` presses the floor beside the avatar: `click bits
0x10000000` then `0x20000000` (a click nothing takes). All scenarios
write `demo world click` / `demo world click result`; the camera's final
focus can be compared with the visible cube/pane. Use the usual `blend-zero`,
`mask-zero`, invisible build/menu and HUD scenarios for neighbouring behavior.

`AURORA_DEMO_TEXREFRESH=1|auto`: a panel right of the avatar wears a texture
whose file in the disk cache is stale, a grey picture crossed in red written
as its `.j2c` before the panel arrives (frame 300), while a local asset
server (the one of `AURORA_DEMO_STREAM`'s `net=`, 150 ms a request) holds
the right one, violet and teal stripes. The panel shows the cross, read from
the disk. "Refresh textures" in its menu (right click), or `auto` at frame
900, forgets the texture, removes its files (`.j2c`, `.part`, `.bc7`) and
downloads it again: the stripes replace the cross (`refreshing 1 textures:
downloaded again` in the log). Capture before with `AURORA_FPS_LIMIT=60`
and `AURORA_CAPTURE_FRAMES=800`, after with `auto` and frame 1300. The
region's `ViewerAsset` is that server for the whole session: the few demo
textures that never had data are answered 404 (`fetch failed` in the log).

`AURORA_DEMO_COLOR_PICKER=1|left|follow|link|toggle|dock|lsl|hex|invalid|drag|cancel|no-apply|palette|pipette`
opens the colour picker of the selection, with
`AURORA_DEMO_BUILD=move,object:118` and `AURORA_DEMO_BUILD_TAB=texture:bp`.

The picker opens against the right edge of the build window, level with its
top. `left` places the build window near the right edge of the screen: the
picker then opens against its left edge. Capture around frame 660. Move the
build window with the picker open: the picker follows, switches sides if
there is not enough room and stays visible. The `follow` mode performs this
drag to the right then comes back to the left: screenshots 800 and 920,
without reopening the colour picker. The `link` mode moves the picker
separately: screenshot 800 for the cone towards the original swatch and its
violet outline, 880 for its fading out on release. The `toggle` mode clicks
three times on the swatch: picker closed at 710, reopened at 750 then closed
at 790. Closing follows the rules of Cancel. The `dock` mode detaches the
picker (760), moves the build window alone (840), re-attaches on the right
(920), checks the following and the switch of side (1000), detaches again
(1080), re-attaches on the left (1160) and checks the following on that side
(1240). The approach must be within 10 points of the edge and of the top;
the snap happens on release. "Apply" is ticked when there is no earlier
choice; unticking it, closing then reopening the picker keeps this choice,
including across a restart.

The LSL / Hex modes change the format through its real buttons; Hex types
`33AA66`, invalid types `GG0000` (OK disabled). Capture around frame 740.
The drag / cancel / no-apply modes drag over 100 frames: capture at 780
during the gesture, 830 on release, 900 after OK / Cancel. The preview
follows immediately, the server receives the final colour on release;
no-apply waits for OK. Palette drags the current preview into slot 32:
screenshot 880 and reopening to check that it was saved. Pipette arms the
tool; click a visible face to take its tint (no change of selection),
release to leave the tool. Escape first leaves the eyedropper, then cancels
the colour picker. The gestures are in coordinates of the real controls,
with no dependency on the saved position of the window.

In Edit mode, holding CTRL and turning the wheel goes through the objects
actually under the pointer: scrolling down to go further away, up to come
back closer, with a return to the start at the end of the list. With Linked
edit, only the prims under the pointer that belong to the same object as the
main selection take part. The gesture replaces the selection without moving
the camera. Over an empty area, the selection stays unchanged.

`AURORA_DEMO_BUILD=cycle,object:99610` adds three nested objects, two of
them linksets, selects A then simulates two notches towards the back and one
towards the front: A → B → C → B. The scenario
`cycle-linked,object:99610` enables Linked edit: root A → part A2 → root A →
part A2, without passing over B or C. The `demo build cycle` lines of the
log give the selections and check that the camera and the HUD zoom do not
change during the gesture. Capture around frame 690 (start 600 by default).

`AURORA_DEMO_BUILD=create-preview,640,430,0` opens Create directly and aims
at nearby terrain without creating an object. The final index selects one
of the 13 primitives (0 cube, 8 sphere, 10 torus); its size is 2 m for a
readable capture around frame 700. Compare `AURORA_REZ_EFFECT=0` and `1`:
both show the same animated loading hologram. Move the pointer, change the
shape and new-prim size, hover the inspector and sky, hold Shift / Ctrl /
Alt, then leave Create: only a valid creation hover shows the preview.
The normal click must still create exactly one object on the surface.
For persistent outlines, `invisible-move,object:99620,z,60,-30`,
`invisible-rotate,object:99620,rz,60,0` and
`invisible-stretch,object:99620,c7,60,-30` install an invisible torus linked
to a mesh, frame it closely, wait for the camera to settle and hold the
manipulation. Capture at frame 780: both outlines remain visible with
highlighting on.

`AURORA_DEMO_BUILD=create-delete,object:118` creates a primitive at
frame 625, selects it after the simulated reply and deletes it at frame 675.
Screenshot at 660: new cube selected; at 700: cube gone and message about
the move to the trash without a false owner warning. `create-ready` leaves
the cube to test its deletion by hand.

`AURORA_DEMO_BUILD=contents-copy,object:118` opens Contents and the
inventory, then copies an item at frame 645. `contents-move` offers the
confirmation of the removal of the non-copyable texture (with a warning for
scripts); `contents-move-plain` shows the case without scripts.
`contents-move-many` selects every item and drops them together on the
inventory: the copyable one goes at once, the two no-copy ones share one
warning that lists them. Screenshot
at 670. The user's warning choices still apply. `contents-ready` opens both
windows to test the drag by hand: copyable script and non-copyable texture.
While holding the mouse button, the native cursor keeps a copy or move
badge over an accepted inventory target (including while motionless), and
shows refusal over a rejected target. Release or Escape restores the usual
cursor; Escape must transfer nothing.
Check copy, cancellation and acceptance of the removal, drop on a folder, an
item and an empty area, and refusal on the outfit / library / Favorites
folders for the texture. On the grid, the human also checks the group
permissions, the contents of another owner and the non-copyable items of
worn objects.

`AURORA_DEMO_BUILD=faces,object:99620` adds three cubes: two linked parts
(A / A2) and a separate object (B). In Materials
(`AURORA_DEMO_BUILD_TAB=texture:bp`), the script selects a face at 625, adds
one on the same prim at 645, one on A2 at 665, one on B at 685, then removes
the first two added faces at 705 and 725. A single click selects only the
top of B at 745. Shift removes this last face at 765: the selection and the
highlighting disappear, and "Select a face" stays available to leave this
mode. Capture just after these steps: a brief highlight marks the faces on
the click, then a violet outer halo identifies them without covering the
materials. The halo follows their visible coverage, without triangle
diagonals or UV seams. `faces-invisible` repeats these gestures on fully
transparent prims: only the selected faces become visible through their
marking. `faces-ready` stops before the clicks for the human test.
Shift + click adds or removes a face, including on another object; removing
the last face of a prim deselects it. Without Shift, the previous selection
is replaced.

`faces-close,object:99620` frames the cube closely then selects a face: the
highlight fades in 0.7 s and only the outer halo remains. Capture at 626 and
700 to compare the brief tint and the fully cleared surface. The eye button
"Show selection again" in Materials restarts the marking.

`faces-complex,object:99620` adds a closed torus, a mesh of which a single
face comprises three curved islands with holes, and a cut hollow cylinder.
It selects the torus at 625 and adds the face of the mesh with Shift at 645.
Capture at 646 then 730: temporary highlight, inner and outer silhouettes of
the two selected faces, materials unchanged after the fade.

`faces-picker,object:99620` selects a face at 625 and opens its texture
picker at 635, with a very long name in a "Project textures" subfolder.
Capture at 660 then 960: the 320 × 720-point picker (limited to the work area) stays bounded without
growing on its own. The selected preview sits above Inventory/Local/Aurora
Library tabs and a virtual folder list. Rows use texture/material icons;
only the selected resource requests a preview. Folder roots can be closed
and stay closed across inventory updates. Presets form two rows beneath the
list (Default/Blank, Transparent/None), followed by a BOM selector for textures
and the validated, vertically centered UUID. BOM shows its placeholder until
selected, then the bake's name; it applies the canonical magic UUID. Local contains the collapsible
"Local files" folder in diffuse texture and PBR material pickers. It lists the
local previews already loaded, textures or materials depending on the choice
requested, and follows the search filter. A click applies the local preview;
"OK" keeps it, "Cancel" restores it. Under the list, Add opens a file
dialog (images, or glTF materials in a material picker) whose files join the
list as local previews; Remove and Import, greyed until a local file is
chosen, drop it or open the importer on it. `faces-picker-local` and
`faces-picker-material-local` open the picker on this tab. To show the
synthetic local files, combine these scenarios with `AURORA_DEMO_IMPORTS=local`.
Keep the picker open while changing the main inventory's name/date order,
"Folders always by name" and "System folders first": its tree and search
results must follow those options, including in the Aurora Library tab.
The inventory image picker, AO animation/notecard pickers, Move to, Replace
links, environment settings and covenant notecard selectors share this order.
Local names compare without case and with numeric runs (2 before 10).
Both scenarios include resources numbered 2 and 10 with distinct dates:
name order puts 2 first, date order puts 10 first.
`faces-picker-material,object:99620` opens the same picker for a PBR
material, with the demo's tiled PBR material selected: the preview is a GPU-rendered
sphere with the material's maps, while list rows remain icons. Drag the sphere
with the left mouse button to orbit it without moving the picker or changing
the selected material; use the wheel to zoom and double-click to reset the view.
Only local materials
must be listed in Local. These files appear without
having been applied to an object beforehand. The import window closes when
the picker opens to clear the view.

In the build picker, a click on a copyable texture, a material, "Default",
"Blank", "Transparent" or "Apply" for the UUID applies the choice
immediately to the selected faces. "OK" keeps this preview without sending
it again; "Cancel" and the close cross restore each face's own texture. A
change of selection also cancels the previous preview. Check on the grid
with several objects and faces that have different textures. As in
Firestorm, a non-copyable inventory texture waits for "OK" and shows this
indication. The profile and terrain pickers keep their usual validation.

`AURORA_DEMO_FLEXIBLE` replaces the objects of the plaza with six prims with
four levels of softness, square / circular profiles, hollow, twist and
non-uniform sizes. The last one follows a rotating root. `rigid` gives the
comparison without simulation; an integer (6–2000) adds prims to measure the
cost. The synthetic wind goes through the same event as the grid. `edit`
isolates a tilted plate for the build tools. To check editing, add
`AURORA_DEMO_BUILD=move,object:90000` or `stretch,object:90000`: pivot on
the base, deformed outline, translucent straight box and size handles.
`stretch,object:90000,fz,100,0` pulls the face opposite the base; the
flexible keeps its curvature during the change of size.

To check the pacing ([#172](https://github.com/Aurora-Viewer/Aurora-Viewer/issues/172)),
use `AURORA_DEMO_FLEXIBLE=1`, `AURORA_NOVSYNC=1`, then the caps
`AURORA_FPS_LIMIT=30|59|60|61|144|500` in separate launches. Gravity, wind,
force and the reaction of the last prim to the moving parent must keep the
same speed in real time; the extra frames smooth the motion. Also check the
`edit` plate (pivot, outline and resizing) and the `300` scene. The unit
tests compare the complete physical trajectories at 30–500 fps, variable
rates, the smoothing between steps, the anchoring, the change of origin and
the catch-up after a long frame.

**`AURORA_DEMO_SCRIPTS=inventory|object|both|save|object-save|error|readonly|syntax|object-syntax|syntax-readonly|indent|object-indent|tabstops|object-tabstops|close-brace|object-close-brace|colors|color-picker|color-accept|color-cancel|comment-lines|comment-block|multiline-indent|multiline-outdent|folding|object-folding|word-wrap|no-word-wrap|word-wrap-preferences|experience|experience-save|workbench|object-workbench|insert-filter|insert-empty|search|search-left|search-inline|search-overlay|search-outside|preferences|help|preprocessor|object-preprocessor|processed|preprocessor-error`**:
opens a script editor from the inventory, the contents of an object or both
at frame 280. `save` and `object-save` save a modified source then close and
reopen the editor at frame 360 with a masked asset UUID, to check the
read-back; `error` simulates a compile diagnostic; `readonly` refuses the
reading of a personal script without the Modify right. Saved sources can be
closed then reopened in the same session. Typing `AURORA_DEMO_SYNTAX_ERROR`
simulates a server error; the demo does not run a local LSL compiler.
`syntax` and `object-syntax` show the colouring of types, variables, native
and custom functions, events, keywords, constants, numbers, strings and
comments. `syntax-readonly` shows the same source coloured and selectable in
read-only mode. The colouring follows the changes while typing and the
colours of the active theme.
`indent` simulates Enter then typing after an indented statement;
`object-indent` does the same with tabs in an object. The added comment must
be aligned with the previous statement.
`tabstops` and `object-tabstops` place the cursor after seven spaces,
simulate Backspace (four spaces), Tab (eight spaces), then typing and Enter.
The comment and the statement must be aligned on eight spaces. Tab adds
spaces up to the next multiple of four; Backspace goes back to the previous
stop when the spaces allow it, without deleting a character of the code to
reach that stop.
`close-brace` and `object-close-brace` close two nested blocks by typing
`}`, with spaces or imported tabs: the closings must be aligned with their
openings. Pasting keeps its indentation exactly.
`colors` shows five swatches next to the numeric vectors between 0 and 1
inclusive; out-of-range position, rotation, string and comment have none.
`color-picker` opens the first swatch and chooses green; `color-accept`
confirms and `color-cancel` restores the original values exactly. The text
stays selectable during the preview; OK makes editing and saving available
and produces a single Undo / Redo action.
`comment-lines` and `comment-block` select two declarations then use Ctrl+K
and Ctrl+Shift+K: `//` is added after the indentation, or the selection is
wrapped in `/*` and `*/`. Repeating the shortcut removes the comments. Block
comments are not nested; use `//` lines if the selection already contains a
block comment.
`multiline-indent` adds four spaces to the selected lines;
`multiline-outdent` removes them. A selection ending at the start of a line
does not include that line; the direction of the selection and Undo / Redo
are preserved. Shift+Tab also removes a literal tab or an incomplete indent.
`folding` and `object-folding` click the arrow of the `default` block in the
margin. Its lines disappear visually, their numbers skip and a label gives
the number of hidden lines. Clicking again or on the label unfolds the
block. The Edit menu contains Collapse all / Expand all. The source stays
identical, including when saving or copying; searching and navigating to a
hidden character reopen its block. The blocks of strings and comments have
no arrow.
`word-wrap` shows a long comment and string wrapping to the editor width;
`no-word-wrap` shows the same source with horizontal scrolling. Resizing or
switching "Wrap long lines" in File > Script editor preferences must leave
the source and line numbers unchanged. The preference is on by default,
including for profiles saved before it existed. `word-wrap-preferences`
also opens these preferences for an immediate toggle and localisation check.
`experience` chooses and saves the simulated experience of an object script;
`experience-save` then closes the editor and reopens it with this
association. The other object demos start with "None". The selector also
allows removing an association then saving. The personal inventory does not
offer an experience association, like Firestorm.

`workbench` and `object-workbench` show the File / Edit / Help menus, the
toolbar, the line numbers, the cursor and the function selector.
`insert-filter` opens Insert, scrolls the results then types `color`: the
filter must stay visible and the source unchanged. `insert-empty` types a
missing name and shows "No matching functions".
`search`, `preferences` and `help` open respectively the find/replace, the
colours/options with preview and the local help of `llSay` with its
signature. The typing commands use the native history; the settings are
never saved on the PC in demo. The file pickers and the external editor
remain explicit actions of the tester.
With `both`, open Find in each editor in turn: the same search window and
its query, replacement and options must follow the selected script. Move
and resize that script: the compact search follows its upper-right corner,
switches to the left if necessary, then overlaps below the title controls
if neither side fits. In that case it stays above its owner when the script
is clicked; another editor can still cover both. Preferences offers two
saved switches: prefer the left side, and embed search at the header's right
when the editor is wide enough. Resize wide/narrow with this enabled:
query, replacement, options and field focus must survive the transition.
The integrated search has two compact rows beside the menus/toolbar and
no close button. The source must start at the same height with or without
search, including after a failed search or replacement status. Its second
row has text buttons in this order: Find, Replace, Replace all. Hover a
truncated status to read it in full.
Drag scripts across the sides and bottom with global edge stops enabled:
they may extend outside, but a usable title grip must always remain.
Opening either script focuses its text before character zero.
Click a script's title, header background, Mono control or footer, then
press Ctrl+F: the shared search must bind that script even without a caret
in its source. Merely hovering another window must not redirect it. Clicking
another window or the world must release Ctrl+F to the viewer's normal
shortcut routing. Repeat between inventory/object editors and from a script
menu or the Insert popup.
In floating and integrated search, keep an existing query, place its caret
in the middle, then press Ctrl+F: the whole query must be selected and typing
must replace it. Repeat from the replacement field and the script itself,
including accented characters and emoji in the query. Enter to find and
resizing between search layouts must preserve a manually placed query caret.
For native captures at 1600 × 1000, `search-left`, `search-inline`,
`search-overlay` and `search-outside` seed these four placements with the
same find/replace values; `preferences` opens the options. Scroll below the
colour preview to see both switches.
`preprocessor` and `object-preprocessor` save a script using an include from
the inventory, a macro, an extended list and a `switch`, then reopen it: the
original text must be kept. `processed` shows the result sent to the
simulator, read-only; `preprocessor-error` shows an error before sending,
with the draft kept.
New `llOwnerSay` messages open Script debug if their routing is enabled; the
errors of owned objects also appear there, after filtering of the bridge and
of blocked objects. The window allows seeing all the objects or a single one
and clearing the displayed messages.

**`AURORA_DEMO_HOVER_TIPS=bubble|card|readout|legacy[:all][:noland]`** — the
world tooltips on for the session in that form (the Legacy Viewer's text
with `legacy`), the saved settings untouched; on land unless `noland`, on
every object with `all`. The simulator answers the parcel under the pointer
(the plaza, the 128 m square in the middle, or the garden for sale around
it) and the objects' properties. Rest the cursor with `AURORA_DEMO_HOVER="x,y"`
and add `AURORA_DEMO_ACTIONS=1` for cubes that can be bought, paid and
touched; capture with `AURORA_FPS_LIMIT=60` near frame 780. Each new tip is
logged (`demo hover tip:` with its title, owner, sale, lines, flags and
buttons). In the default demo view, `550,500` is the Pay cube, `752,420`
Orion, and with `AURORA_DEMO_POS=40,126` the ground at `800,600` is the
garden.

**`AURORA_DEMO_ACTIONS`** — cursors and object clicks like Firestorm
(same rules; cursors drawn from the Tabler icons).
No real L$ and no connection to a grid.

- `1`: all the objects, for a manual test.
- `sit`, `buy`, `pay`, `none`, `touch`, `disabled`, `ignore`, `open`,
  `play`, `pause`, `open-media`, `zoom`, `grab`: one isolated target; hover
  and click through the usual path.
- `none` / `touch` send press, move and release; `grab` simulates the
  physical move.
- `ignore` goes through a visible screen and opens Buy behind it; `disabled`
  stops the click without touching; `ignore-build` selects this screen in
  build mode.
- `open` shows three simulated content items.
- `open-multi` (#1410): the Open window receives Ctrl+A at frame 420;
  every contents row is selected together. Ctrl/Shift clicks, a selected-row
  drag or its Copy/Move menu act on the selection; Delete removes selected
  entries only on a modifiable object. The list accepts inventory selections
  and folders, including when empty; a refused entry refuses the entire drop.
- `open-copy`, `open-replace`, `open-add` (#1103): after `open`, the
  window's "Copy to inventory" at frame 420 (`replace` / `add`: the "Copy
  and wear" menu's "Replace outfit with contents" / "Add contents to
  outfit"). The cube is ours and its "Spare cube" is no copy, so the move
  warning opens and waits; add `-yes` to confirm it at frame 480. The
  inventory opens at frame 540 on a new folder named after the cube (under
  Clothing for `replace` / `add`) holding the three items, and the cube's
  contents lose the spare cube. The log has one `demo contents copy: 3
  item(s), 1 moved` line; for `replace` / `add`, then `contents copy: all
  items arrived, wearing the folder`. Wearing needs a Current Outfit folder:
  add `AURORA_DEMO_APPEARANCE=worn` (the cube is then linked in the outfit;
  the demo does not attach it).
  After confirmation (`open-copy-yes`), the inventory footer confirms the
  three actual arrivals, then clears after five seconds (capture at frame 650
  for success, 1050 after expiry, with `--fps-limit 60`).
- `open-copy-timeout-yes` (#1410): the offline simulator withholds the spare
  cube; frame 600 advances the copy observer's deadline. The footer reports
  an unconfirmed copy with two of three items received, then clears after
  five seconds. This fixture changes neither real-grid transfers nor their
  60-second deadline. The move warning still requires confirmation at 480.
- `play` starts a local HTML page; `pause` and `open-media-playing`
  inject a simulated playback state before the click; `open-media` simulates
  the opening of the external browser.
- `zoom` frames the cube.
- `tp-door`: the Touch cube answers like a club's teleport door scripted with
  `llMapDestination` (ScriptTeleportRequest): the world map opens centred on
  a tracked spot of the plaza region, at 118 m; nothing teleports until the
  map's Teleport button. `tp-door-ready` stops before the click.
- `buy-confirm`, `pay-confirm`: confirm the simulated transaction.
- `pay-menu`, `pay-linked-menu` (#997): open Pay through the context-menu
  action on a Touch object, or its linked child (only the root takes money).
  The normal click action stays Touch; suggested amounts must still load.
  Add `-confirm` to pay L$ 10 at frame 320. The log must contain one
  `demo payment confirmed` with the clicked prim's UUID (972 or 974),
  `dialog=false` and one `demo action: paiement L$ 10`; the window closes.
  Without `-confirm`, the window stays open for manual input. Check both
  Pay window looks in Preferences > Aurora. No real currency is transferred.
- `linked-touch`, `linked-sit`: hollow root with Touch and child seat with
  Sit, overlapping bounding boxes; checks the prim actually aimed at.
- `probe-sphere`, `probe-box` (#1401): an invisible reflection probe encloses
  the scripted Touch cube. The click must reach cube 975, with a matching
  `demo click action: Touch, object=975` and `demo probe click: ... held=false`
  after release in the log. Add
  `-ready` for manual clicks and the frame-650 hover sample: it must report
  `prim=Some(975), action=Some(Touch)`. Combine with `AURORA_HOVER_CHECK=1`;
  probe selection remains available through the build tools' explicit toggle.
- `transparent-menu`, `transparent-build`: cube with 60% transparency,
  without a Touch script or click action; right click opens its menu and
  left click in edit mode selects it. `invisible-menu`, `invisible-build`
  repeat these gestures with 100% transparency and no draw record.
  Capture after frame 620 and inspect `demo action click`: selection must
  contain prim 976, and `demo menu` must target that prim.
- `transparent-ready`, `invisible-ready`: the same cubes without an automatic
  click, for manual testing (near x=138, y=126).
- `blend-zero`, `blend-faint`, `blend-half`, `blend-solid`: non-actionable
  panel before a Buy object, with texture alpha 0, 1/255, 128/255 or 255/255.
  `blend-zero` must pass the left click to Buy (971), with `dialog=true`;
  the other three must reach panel 976 and leave `dialog=false`. The GPU
  response must omit the zero-alpha panel but retain the faint/half panels.
  `blend-zero-menu` must still target panel 976 with the right click.
  `blend-backface` replaces it with a cube whose transparent entry face
  reveals Buy while the opaque exit face is invisible from this camera.
  Its shadow must not prevent Buy (971) from receiving the click:
  `dialog=true`, including with shadows disabled. The CPU fallback must
  reject that back face just as the GPU does.
  Back-facing surfaces must also be ignored by hover, menus, build selection,
  placement and media, regardless of their texture or alpha mode. On the grid,
  repeat with a cutout cube in front of a scripted box and replace its unseen
  exit faces with Blank: they must not intercept the interaction. Explicitly
  double-sided PBR faces remain targets on both visible sides.
  `mask-zero` uses the zero-alpha texture in Mask mode:
  the click must reach Buy (971), with `dialog=true` and no panel in the GPU
  response. Capture after frame 620.
- Add `-ready` to the Blend / Mask modes above to test hover without
  an automatic click. At frame 650, `demo cursor hover` must report
  `prim=Some(976), action=None` before Blend at every opacity, and
  `prim=Some(971), action=Some(Buy)` through the Mask hole. Enable
  `AURORA_HOVER_CHECK=1` to compare the cache and index with the full search;
  media hover behind Blend must respect the same depth limit.

Transaction windows, with the same variable:

- `pay-layout`, `pay-hidden`, `pay-large`: four four-digit amounts, free
  field and hidden buttons, maximum amount.
- `pay-vendor`, `pay-vendor-short` (#1869): a vendor with one L$ 180 price
  and no free amount, like CasperVend. The price button pays at once (no
  Pay button below it); `-short` starts with L$ 100: the button stays
  enabled and opens Buy L$ for the missing L$ 80, the window staying open.
- `buy-touch-menu` (#1869): the for-sale cube keeps the Touch click action
  and Buy is chosen from its menu; add `-confirm` to press Buy at frame 320
  (the log shows `demo purchase confirmed: … prim=971, price=10`).
- `buy-original`, `buy` (copy), `buy-contents`: purchase of an original, of
  a copy or of the contents, list of the included items and permissions of
  the next owner.
- `buy-empty`: offers contents that cannot be sold and disables the purchase.

**`AURORA_DEMO_DIALOG=12|4|long|links|scripted`** — llDialog menu of twelve answers in
three columns, arranged from the bottom to the top like Firestorm; `4`: four
answers, to check the incomplete last row; `long`: long label truncated,
with the full text on hover. Block and Ignore buttons under the menu.
`links` (#898): a menu whose text lists two residents and a group by their
`secondlife:///app/.../about` links and a region by its `app/region` link;
they show as names (a person or group mark before them, the region with its
badge), clickable. With `AURORA_DEMO_NOTIFS=legacy` and a capture before
frame 700, the same menu in the Legacy toasts. `scripted`: every window a
scripted object sends at once (an llDialog menu, an llTextBox, an
llRequestPermissions question, an llLoadURL link and an object's
llGiveInventory), then four pieces of information. The windows stay out
until answered, in both notification styles: the newest three shown, the
others behind "+ N more", the information folding away after its time;
open and close the list, or go into mouselook and back, and they come out
again.

**`AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`** — script of the build
tools. Modes: `move`, `rotate`, `stretch`, `face`, `align`, `grab`,
`focus`, `create`, `land`, `select`. `AURORA_DEMO_BUILD_FRAME` changes its
start frame.
`shared,object:99620` and `shared-denied,object:99620` select another resident's
linkset with granted or denied effective Modify rights. Use
`AURORA_DEMO_BUILD_TAB=texture:bp|texture:pbr|shape|features|general|rights` and
capture after frame 700. Granted rights enable surface/property edits and
report an editable object; sale and permission controls remain disabled.
Denied rights leave the object movable but its surfaces/properties read-only.
The variant `mode,object:<local id>[,part,dx,dy]` selects an object of the
demo without fixed click coordinates.
`megaprim,object:118` makes the selected object a 128.125 × 96.25 × 0.5 m
platform below the plaza. With `AURORA_DEMO_BUILD_TAB=object`, verify that
opening Edit retains these dimensions in both construction presentations.
Moving or rotating must retain its size; deliberate size edits keep the normal limits.

**Seated object editing:** combine `AURORA_DEMO_CAMERA=sit`,
`AURORA_DEMO_WORN=1` and `AURORA_DEMO_BUILD=linked-click-ready,object:950` (whole seat) or
`linked-click,object:950` (root in linked editing); capture at frame 690,
before the avatar stands at 700. Only the seat's prims are outlined (two in
whole-object mode, one for its linked root), without the avatar or backpack. The selection
regressions also add HUDs and seating on a child prim; the transform regressions
cover move/rotate/stretch on the root and child in both selection modes,
checking occupant/attachment local coordinates and outgoing transform targets.

**Mesh Shape inspector:** `AURORA_DEMO_BUILD=mesh-inspector,object:99621`
opens the native Shape section on a mesh with four different generated LODs.
The counts come from its actual geometries. Capture after frame 710.
Suffixes `-high`, `-medium`, `-low`, `-lowest` force that LOD locally; compare
the visible geometry and active row, then restore Default. `-readonly` keeps
the information/menu available without Modify rights. Link/face numbers belong
between the respective footer arrows; select multiple faces to check the list
and its full hover text. Inspection resets on selection/asset/tool changes or
closing construction, stays across editing sections, never changes object
data/global preferences and keeps HUDs at high LOD.
`mesh-inspector-cycle` switches lowest/high/low/medium/default at frames
630/655/680/705/730 with the camera stationary. Capture each stage just before
the next transition. `mesh-inspector-cycle-empty` makes lowest a real empty
geometry: capture at 645 (no faces), 670 (high restored), and 750 (automatic
restored). Returning from the empty level must not wait for camera movement,
object updates or the periodic LOD sweep.

**`AURORA_DEMO_BUILD_FOCUS=off|on`** — focus on the selection before taking
a handle, with "Follow object while editing" disabled / enabled. With `off`,
the view stays fixed at the centre of the screen; with `on`, it follows the
local transformations without jolts or drift when the mouse stays still.
Test moving and stretching by face and by corner.

**`AURORA_DEMO_BUILD_EDGE=left|right|top|bottom`** — completes a `move`
script: the mouse reaches the edge in 40 frames, stays there 60 frames then
releases the handle. With following disabled, the camera slides gradually
near the edge on the same axis or in the same plane as the object, which
stays visible while the mouse is held, without gradually moving closer or
further away. Test an axis oblique to the camera to check this distance.
Capture around the start frame +120 during the hold and +170 after the
release; the camera must then be stopped. Combine with
`AURORA_DEMO_BUILD_FOCUS=off` to check the focus on the object, then without
this variable to check the initial focus on the avatar. The HUDs and enabled
following keep their behaviour.

**`AURORA_DEMO_BUILD_ECHO=<ms>`** — the demo's report of each transform edit
(MultipleObjectUpdate) arrives that many milliseconds after it was sent, like
a simulator a round trip away; 250 is a typical value. Stretch an object
quickly with Ctrl+Shift and release: it must stay at its final size and
place, without going back to an earlier size for a moment (issue #1193).
Moves, rotations and the Edit window's fields must not jump back either.

The demo also answers the build tools' Undo and Redo (Ctrl+Z, Ctrl+Y) like a
simulator: each object keeps its placements before each edit, the updates of
one drag (closer than 0.4 s) making one step.

**`AURORA_DEMO_BUILD_TAB=general|rights|object|transform|shape|features|texture[:pbr|bp|bp-normal|bp-specular|media]|contents`**
— section of the build window shown by the script. General includes object
permissions; `rights`, `object` and `transform` redirect to General for older
scenarios. `+weights`, `+media` and `+permissions` open the related dialogs;
`+grid` opens the side settings with inline ruler controls.

Use `faces-ready,object:99620` with `texture:bp-normal` or `texture:bp-specular`
to inspect and edit UV transforms before assigning either map. Assign a map
afterwards: its prepared scale, offset and rotation must remain. In every
edit section, three coordinate icons beside Edit linked parts select World /
Local / Reference directly, with exactly one highlighted. Choosing Reference
uses the selection when no grid reference exists; switching sections and
clicking the active icon retain the mode.

For material number entry, use `faces-ready,object:99620` with `texture:bp`,
choose the default specular map, then type shininess/environment values one
digit at a time. Enter or leaving the field applies the whole number; Escape
cancels it. The inspector must keep its size, controls and focus while the
simulator's replacement material loads. The native inspector regressions also
simulate ObjectUpdate arriving before RenderMaterials.

For PBR scrubbing, apply a material and drag the metallic/roughness number in
both directions, hold still, release, then wait at least three seconds. The
inspector must retain its chosen width, including with text enlarged to 140%.
Its edge/corner handles must still resize it normally; changing sections and
folding/reopening the panel must retain that manually chosen size.

Use `AURORA_DEMO_BUILD=contents-properties,object:118` with the Contents
section to open a task item in the same properties panel as agent inventory
(name, picture, owner/creator/date, description, masks and sale). Task items
have no thumbnail metadata, so their Picture action is disabled. Saving must
update the original object's item even after changing the build selection.
With `AURORA_DEMO_BUILD=contents-ready,object:118` and
`AURORA_DEMO_BUILD_TAB=contents+permissions`, check the eleven icon filters,
All/Clear, Group/Anyone/Next owner columns and OK/Apply/Cancel. No-copy forces
transfer; environment settings retain copy. Apply leaves the dialog open;
OK closes it while the queued requests finish. Cancel restores the opening
choices without starting another update; requests already applied continue.
Automated regressions check each type,
inventory fetching, revoked access, failed reads/timeouts and task saves.

`AURORA_DEMO_BUILD=contents-multi,object:118` shows two selected task items;
`contents-rename-ready,object:118` leaves the first item's inline rename open.
In Contents, plain click selects one entry, Ctrl toggles entries, Shift
selects a visible range and Ctrl+Shift extends it. Ctrl+A selects the visible
list; right click on a selected entry preserves the batch. Delete removes
only the selected task items while this list owns focus, including after
repeated presses with an empty or read-only selection. It must never send
the edited object to trash. Clicking in the world releases list focus and
retains the object's normal Delete shortcut. F2 requires one modifiable item.
Rename through F2 or the menu: Enter, clicking another row/field or leaving
the tab commits once; Escape cancels. Delete inside the rename field edits
text. Filtering removes hidden entries from the selection. Native inspector
regressions cover these pointer/key flows and the task inventory commands.

The object inspector resizes in both directions. Combine the build scenario
with `AURORA_DEMO_BUILD_FRAME=400` and
`AURORA_DEMO_RESIZE=drag,build_tools_inspector_c3` to exercise its real
corner handle and capture at frame 780; the footer must follow the new
bottom/right edges. Each Position/Size/Rotation caption has its own copy/paste
pair. Linked editing, face selection, both-sided stretching and root-axis
editing remain in the shared footer throughout the seven Edit sections.
With `AURORA_DEMO_BUILD=contents-ready,object:118` and
`AURORA_DEMO_BUILD_TAB=contents`, the inventory surface fills the remaining
panel down to the shared edit footer and expands after a corner drag; the
filter must preserve this surface height even when only one item matches.
With `AURORA_DEMO_BUILD=grab,object:118`, capture at frame 1100: the wrapped
move-tool hint must stay inside the chosen window height, without the window
growing on successive frames. The native layout tests also exercise a longer
wrapped status message and a 64-item object inventory at two heights.

**`AURORA_DEMO_LSL_BRIDGE=1`** — messages of the Firestorm LSL bridge
(hidden) between two "owner say" lines.

**Bundled Aurora bridge scripts:** Rust embeds the `.lsl` sources as text;
`check.ps1` and the offline factory do not compile them. Type-check all three
scripts in Mono mode with `lslint` when changing them:

```powershell
foreach ($script in 'bridge', 'query', 'ik') {
    lslint -m "crates/aurora-viewer/src/world/bridge/$script.lsl"
}
```

Execute their actual event handlers offline with
`uv run scripts/test-bridge-lsl.py` (isolated Python 3.10/Lummao test runtime).
The test covers silent startup/URL failures, exact owner/root discovery,
login revocation, confirmed-session region renewal, 60-second self-detachment, permission checks,
unpaired peer/HTTP traffic and resumed discovery. It mocks simulator APIs;
it does not emulate Havok or measure the compiled Mono memory budget.

Bridge 1.40 also checks container normalization at startup and reattachment.
On the grid, let Aurora upgrade an existing bridge, then inspect its build
properties: a 0.01 m cube on all three axes, no cut/hollow/twist/taper, default
plywood texture on every face and 100% transparency. Its old landscaping
description must be empty. Detach and reattach it, then relog: the same object
must retain these properties and its bridge services. A fresh bridge must
finish provisioning with the same appearance.

For bridge 1.38 (#1168), let Aurora upgrade the existing attachment, then
switch to another viewer without manually removing it. No bridge message
should appear at login or after crossings/teleports; the bridge must detach
after 60 seconds without Aurora discovery, even during repeated crossings.
Reconnect with Aurora: the same inventory object must be worn automatically,
all three services must recover, and Movelock, IK, script/object information,
posing consent and portals must still work after crossings and teleports.

Existing-bridge reuse: reconnect to Aurora while the current bridge is still
worn. It must verify all three services without compiling scripts or temporarily
detaching the object. Reconnect after the other viewer detached it: there must
be only the necessary attachment, with no second save detach/reattach cycle.
An actual bridge update or explicit Repair still compiles and saves once.
The factory regressions count script writes, attachments and detachments for
both reuse paths, each service's version mismatch and an unfinished update save.

Bridge 1.35 repairs four invalid unary `!` operations on keys in the query
script's posing-consent paths. The human grid check is to let an existing
attachment upgrade, confirm all three scripts are running, then request Script
information and Object information on a rezzed object and an attachment.
Confirm the reports appear in nearby chat and exercise posing consent between
two clients. The LSL check supplements actual simulator compilation and runtime
validation.

**Bridge IK relay:** `AURORA_DEMO_MOVELOCK=1` exercises the three-script
attachment factory and movement controller; capture at frame 1600 after its
save/detach/restore stage. `AURORA_DEMO_BRIDGE_QUERY=radar` checks the independent
read-only service at the same frame. The bridge simulator verifies installation
and upgrade of the third IK script plus exactly one discovery per relay session.
Headless `world::hand_ik::relay` tests cover own/observed seats and attachments,
inactive preloading, state-only profile transitions, an hour without renewal
requests, private session replacement, source removal/reappearance, late avatar
parentage and bounded identity-only declarations. The usual moving/observer
and IK Edit reload scenarios remain direct offline rendering fixtures. Real
LSL compilation and regional announcements across two viewers require human
grid validation with the current complete publisher export.

**Native Movelock choice (2026-10-18):** `AURORA_DEMO_MOVELOCK=native`
opens the same options with native sitting selected and an offline full-body
P4 standing AO above a P3 constrained sit. `native-missing` disables Aurora's
integrated AO: the existing bridge lock takes over without native sitting.
The selected native method stays unchanged. The bridge choice says "classic
method"; native sitting has no explanatory paragraph. Its fallback status names
Aurora's integrated AO, requires at least one compatible standing idle animation
and says the classic system will be used otherwise.
`native-fallback` starts anchored, disables the integrated AO at frame 700 and
restores it at frame 1000. Capture at 620/850/1250: native sitting, bridge
fallback after stand-up, then native sitting again without changing the method.
`native-moving` simulates continuous voluntary input: the selected option
stays enabled while its status reports free movement. Capture after frame
620. The bridge modes below remain available and the bridge stays the default
for existing/new accounts. The choice is saved per avatar; right-click the
toolbar lock to open these options.
The optional lock indicator is off by default. Enable it in the options,
drag its body to another position, hide/show it and restart: the saved UI
layout must retain its position. A click without dragging still opens the
options; shrinking the viewer keeps the indicator inside the work area.
The indicator says "Position lock active" ("Verrouillage actif" in French)
for the classic method, native hold and bridge fallback; transient states
retain their compact labels. Hover it for the full status.

Offline checks exercise control edges, bounded retries/late replies, manual
seat ownership, region changes, per-channel priorities, complete standing
cycles and observer animation commands. Demo sitting replies are synthetic;
they do not model Havok push resistance. Human grid acceptance must compare
both methods: enable a suitable standing AO, stop/move/run/jump, walk/run to a
double-clicked point, hover in air,
switch/disable the method, sit on the ground/on a chair, cross a region and
relog. Confirm with another viewer that standing cycles remain visible and
that scripts still report sitting. Check physical/scripted pushes while
anchored, then swim/walk on water and return to land: native sitting must
pause for these water controllers. It requires Aurora's active integrated AO
whose looping standing selections override every constrained-sit channel at a higher joint
priority; missing, empty, disabled, paused, downloading or incompatible standing
sets use the bridge fallback. Disable the integrated AO while anchored, enable
it again, then remove/restore its standing idle: each loss releases the native
sit and enables the bridge, and each recovery resumes native sitting. A worn
third-party AO alone cannot meet the native method's requirement. Check the
bridge's ordinary movement/rearming and manual-seat pause during fallback too.
No animation is imported/uploaded, and no seat object is rezzed.

**Native rearming (#632):** `AURORA_DEMO_MOVELOCK=native-rearm` repeats the
native walk/stop cycle with a stale Falling signal at rest from initial
activation onward. Capture at 550/900/1300: all three stops must be anchored.
On the grid, enable native sitting at rest, then repeat short
walk/run taps, jumps/landings, and water-to-land transitions. Include a dry
platform near the waterline with swimming enabled. A stale falling/finished
landing signal must not prevent anchoring on physical support. Once the
simulator confirms sitting, its height change or old airborne signals must
not release it. Deliberate movement, active swimming/water walking, manual seats
and region arrival still have priority. The options distinguish waiting for
region arrival, water movement, airborne motion and another movement controller;
`native movelock:` log lines record each status change without identifiers.
Compare push resistance and observer standing cycles on the grid; offline
controller tests do not simulate Havok.

**Native observer transitions:** `AURORA_DEMO_MOVELOCK=native-cycle` uses
the same options and complete standing fixture, walks during frames 600–799
and 1000–1199, and stops otherwise. Capture at 700/1100 while moving and
900/1300 after reanchoring. Native sitting waits for the real AO batch's
synthetic simulator echo and authored easing; custom AO clips survive the
native state reply. The enabled preparing status precedes the confirmed lock.
After releasing movement, preparation must wait for simulator velocity and
local visual settling to fall below the ordinary 0.5 m/s walk-stop threshold.
Offline regressions cover delayed walk/run stops, short taps, legacy position
smoothing, Aurora's drawn glide, vertical flight and push reports while locked.
For a native-window check with delayed simulator reports, combine this mode
with `AURORA_DEMO_LINK=200,stoplag=300,aurora,trace` (or `firestorm` for legacy
movement). Inspect the end of each walk before the 900/1300 reanchor captures.
The local regression feeds raw P3 sit/stand-up plus P4 standing/walking clips
to a separate production motion controller, rather than the owner's filtered
pose. It checks entry and unequal ease-in/ease-out handover for a pelvis dip,
as well as late movement signals, standing cycles and missing replacements.
Human observer acceptance remains required: have another viewer watch repeated
walk/run starts and stops, including short taps and turning, while confirming
the anchor still resists pushes at rest. A cold observer's animation download
cannot be acknowledged by this viewer; this fixture tests known loaded clips.

**`AURORA_DEMO_MOVELOCK=on|off|moving|seated|error|timeout|endpoint|halted`** — Aurora's bridge and position lock. The demo seeds the library object, runs the nested inventory creation/copy/attach/script-upload/save callbacks, opens the options and shows the requested activation immediately in the bottom toolbar (the bottom-left indicator is off by default; tick it in the options); provisioning remains a separate diagnostic. `moving` and `seated` change physical maintenance internally while the button, enabled label and indicator remain active; `error` simulates failed compilation; `timeout` withholds the first hello and reports the script-startup timeout after 60 seconds; `endpoint` supplies an untrusted address and must show a refused connection response without exposing its URL. `halted` stops the script once it is ready, as a Stack-Heap Collision does: every request then times out; about ten seconds later the viewer finds the script not running, says it is reinstalling the bridge, recompiles it and saves it again (detach/reattach), and Movelock returns without any action. After three automatic reinstalls in ten minutes it stops and leaves Repair to the user. The demo installs both scripts of the bridge (controller and read-only query service) and answers the discovery probe with both hellos. A real bridge also reports the controller's lowest free memory with each status: the log line `script memory low mark` shows the margin left in Mono's 64 KB. The normal fixture uses the current secondlife.io simulator domain. Click the lock icon or press Ctrl+Alt+P to toggle; its right-click menu opens the options. Settings are saved per avatar. The human must validate real LSL permissions, physical bump resistance, movement/sitting, relog and region changes. Confirm a single existing object is reused/moved into inventory root → `#Aurora` → `Aurora Bridge`, with `#Aurora` sorted above system folders. The stable object/script name remains `#Aurora Bridge v1.0`; the installed protocol is 1.8. The factory waits for the new script to respond before detaching, then five seconds after detachment before reattaching, without requiring an inventory asset UUID update. A lost hello is repeated until confirmed; a stopped script can be started once. Startup, movement permission, simulator connection, inventory save and rejected responses have distinct progress/timeout messages. HTTPS must use the same simulator TLS trust as the other capability requests; redirects remain disabled. on_rez must preserve the queued attach event. Readiness must follow the real TAKE_CONTROLS grant without a script-debug error. The visible enabled state follows the user selection even while physically moving, seated, being pushed, waiting for acknowledgements or encountering a connection error; diagnostics remain in the options. Physical convergence (within 0.35 m of the anchor and below 0.5 m/s) remains internal. Region changes keep activation enabled; the obsolete region-off option is ignored. Movelock has no timeout/keepalive: it must still resist pushes after a long gap without viewer HTTP, until explicitly disabled. LSL control events alone release/reanchor the hold. Movement always releases the hold through simulator control events and relocks at the new position on release; all movement/nudge keys remain available. Old hard-lock settings are ignored. Toggling from the toolbar, shortcut or options emits no dedicated Movelock sound. Detach it explicitly: Movelock turns off, enhanced movement pauses and the bridge stays detached for the current session until enabled, worn or repaired again. The internal installation save must still reattach after its delay. Check explicit unlock and cleanup at logout; no elapsed timer may disable Movelock. Demo validates lifecycle and UI; it does not execute simulator physics. The human validated Movelock on 2026-10-15; the new movement motors retain its hold/reanchor functions.

**`AURORA_DEMO_MOVEMENT=surface|underwater|breaststroke|backstroke|crawl|independent|float-low|float-high|off|run-fall|run-ao|run-drop|passage-blocked|passage-reached|passage-pending`** — Enable the movement aids and raise synthetic water at frame 360; `underwater` raises it further, and the three named stroke modes show the contributed local poses. Ctrl+Alt+M opens the AO Movement tab: running and flight use x1..x5 factors, with native x1 as default. Passage requires its own switch and Ctrl plus the configured jump key. Native flight is disabled on swim entry and its previous state is restored on ordinary exit. Vertical controls reach LSL only after capture acknowledgement; fast-up stays absent during swimming/controlled exit. The ordinary AO owns swimming choices and broadcasts inventory animation assets; private local IDs must never be broadcast. The six bundled priority-4 loops remain available locally without upload. No pack uploader or automatic swim-state seeding remains. The ordinary AO uses inventory assets, while swimming assignments belong to Aurora’s own set.

Water gestures (2026-10-17, bridge 1.23): use the configured ascent/descent actions (PageUp/PageDown by default). Tap descent at the surface: complete a physical dive after release and retain depth. Tap ascent underwater: return to the adjusted flotation height; holding/releasing that same press must never start flight or jump on arrival. Repeated ascent presses continue the return; descent cancels it. Current surface gesture: a single ascent press jumps, whether stationary or moving; at a solid shore with a safe landing it uses the rise-then-advance jump. A second distinct press within 400 ms requests higher flight when allowed; holding a key must not request flight. A no-fly parcel still allows a surface jump. Check the correct surface/underwater forward AO list across ascent and idle, short/tall avatars and float offsets from -0.5 to +0.5 m. Bridge 1.23 increases the surface return command to 3-5 m/s with stronger acceleration and braking near the goal; these are command limits, not measured Havok speeds. Inspect actual ascent duration and overshoot on the grid. Ceilings/occupied landings must stop the physical target; sitting, suspension, disable, bridge loss and timeout release pending phases. Check remapped keys, native/enhanced flight handover, shallow/deep water and automatic shore exits. Offline unit tests, LSL lint and prescribed native trajectories verify controls/animation continuity; they do not simulate Havok.

Physical passage experiment (2026-10-16, bridge 1.16): human feedback confirms that 1.15 executes the action but fails at a wall (0.60 m progress, 1.31 m remaining, highest sampled speed 6.0 m/s, 11 impulse requests). At the maintainer's request, the new motor uses the accepted Movelock physics with its anchor at the bounded far-side target: SLED, linear/angular friction timescales 0.05 and MoveToTarget(target, 0.05). It changes no normal Movelock setting/anchor and does not capture keys. No impulse or velocity assignment is used. Verify whether this attraction crosses the same simple wall; record observed progress, remaining distance and sampled velocity, rather than command receipt. Impulse count is no longer displayed. Arrival or expiry must remove the temporary target/SLED, while enabling normal Movelock during the attempt must preserve the new normal anchor. Also check cancellation on disable, sitting, focus loss, region change and detachment; replay cannot retarget or renew the deadline. The server lifetime remains 1.2 s, viewer delivery allowance 1.5 s. The passage-blocked/reached/pending fixtures remain synthetic UI/protocol states; they do not simulate Havok or establish wall penetration.

The target sensor/completion radius is 0.10 m so the avatar's conservative body clears the far side before release; the previous 0.35 m completion radius could stop inside the wall despite a nominally clear goal. Actual-LSL checks cover that premature-stop case, normal arrival/overshoot and arrival observed after the deadline. On the grid, ensure that an apparent crossing is not immediately pulled/pushed back by an overlapping capsule.

Offline checks cover waterline hysteresis, diving without involuntary surfacing, shallow-water exit, blocked steep-edge jumps, cooldown, diagonal speed limits, animation stop/start, lock/sit/suspension release, stroke serialization and conservative physical-passage bounds and one-shot action lifetime. LSL static analysis can be run with [lslint](https://github.com/Makopo/lslint); this checks syntax and semantics, not Havok behavior. Later grid validation must cover actual bridge provisioning and physical target effectiveness, visibility of AO-selected inventory swimming animations to another resident, return to walking on a beach, underwater obstacles, steep exits and physical passage through thin obstacles and a blocked attempt against an impassable obstacle. Linden region water is detected; decorative prim water is not classified automatically. The human requested offline progress without their grid tests for now; these points remain unverified.

Movement motor follow-up (2026-10-15): protocol 1.12 sends motor/height settings, not world-space thrust vectors. Verify running with quick turns/reversals and releasing keys, flight acceleration/idle braking (25 m/s default), surface entry without enabling flight, underwater idle/depth/up/down and a return to shore. Ctrl+Alt+G requests a bounded physical target beyond a nearby wall; no TeleportTo or local avatar relocation is sent. Check one passage only per press, stopping when blocked/expired and no replay on repeated HTTP requests. Offline checks cover payload limits, direction-independent motor settings, passage cancellation/lifetime, migration and the existing Movelock regressions; LSL lint does not execute Havok.

Camera orientation follow-up (2026-10-15): run/swim/ordinary-flight steering uses camera yaw through llGetCameraRot; mouselook flight also uses camera pitch. TRACK_CAMERA is requested with TAKE_CONTROLS in one call and is optional for Movelock readiness. Check a camera heading different from the avatar, quick turns while holding forward, left/right/backward and diagonals, mouselook flight looking up/down, level swimming with a pitched camera, and shore/passage probes toward the viewed obstacle. A vertical camera must leave horizontal probes finite, using avatar heading. The viewer already sends camera axes in AgentUpdate, as Firestorm send_agent_update does; no HTTP camera updates are introduced. Offline regression tests cover native probe heading/pitch/vertical fallback; real LSL camera tracking and physics need human grid validation.

Independent movement controls (2026-10-15): `independent` shows x3.5 running enabled with flight/swimming disabled; `float-low` / `float-high` show -0.35 / +0.35 m flotation. The three switches retain their own settings; turning off one must not turn on a different motor. x1 uses entirely native running without a scripted target; factors above x1 up to x5 boost only running, and no factor below x1 is accepted. The script uses nominal run speed rather than multiplying already boosted velocity; faster forward motion must not be braked. Flotation offsets range from -0.50 to +0.50 m in 1 cm steps, zero by default. Verify stable surface idle at both bounds, no involuntary dive/surface after an adjustment, diving/returning to the adjusted surface, and immediate release/animation stop when swimming is disabled during an exit pulse. Offline tests cover every switch combination, multiplier bounds/native x1, offset classification/payloads, cancellation, persistence/migration and protocol limits. Physical speeds/flotation still need human grid validation.

Independent activation/grounded running (2026-10-15): the global activation row is absent. `off` keeps every enhancement and the toolbar indicator off; any individual enabled switch lights the toolbar even with the options closed or bridge replies missing. Old global-off preferences migrate to all off. `run-fall` supplies short raw falling signals while enhanced run remains selected, `run-ao` also loads the synthetic Aurora AO set, and `run-drop` supplies a marked/long fall that must release the run pose. These are animation/state fixtures, not Havok simulations. Running uses positive horizontal impulses with no altitude target and no impulse while airborne/jumping. Protocol 1.9 closes the requested velocity gap directly instead of limiting it to 30 m/s² per timer tick, and relies on native collisions without a look-ahead ray veto. Compare the measured simulator speed at x1, x2 and x5 on clear flat ground; a laggy ray budget must not disable the boost. On the grid check flat ground, slopes, raised prim platforms, stairs, releasing keys, a real jump and running off a ledge; no persistent run-in-air or hover. With Aurora AO, its chosen run/cycle should survive brief contact jitter. With Aurora AO disabled or no usable running clip, the standard run is broadcast; unrelated gestures/foreign assets and raw transition signals stay intact. A scripted third-party AO can still override that standard fallback. Landing/pre-jump finish controls remain available; no new LSL animation permission or persistent server animation override is used. Verify remote appearance with a second resident; only the human connects to the grid.

Swim defaults follow-up (2026-10-15): fresh preferences use 5 m/s. Existing 2 m/s defaults become 5, custom speeds remain; changes made after migration survive another load. Native animation broadcasts remain enabled.

Sustained run/feedback follow-up (2026-10-15): the human measured about 5.1 m/s at x1 and only occasional 5.4 at x5 with the v1.9 impulse motor. Protocol 1.10 replaces it with a bounded horizontal continuous-force regulator following the Firestorm flight-assist llSetForce/llGetMassMKS mechanism, with local velocity feedback and anti-windup. Verify x1/x2/x5 on clear level ground and compare the native measured speed with the new actual script report (received target, speed, run state and cumulative thrust updates). A received setting alone is not proof of physical acceleration. Check immediate force release on key-up, jump/fall/flight, sitting, Movelock, a mode switch, a lost command watchdog and explicit detach; no vertical hold or residual thrust. Offline function evaluation checks control flow/cleanup and a deterministic braking model, not Havok or its actual constants; the real grid effect remains unvalidated.

Smooth run follow-up (2026-10-15): the human confirms v1.10 accelerates but is jerky and excessive already at x2. Protocol 1.11 keeps continuous horizontal force with legacy llGetMass scaling, ramps the target by at most 4 m/s per second and upward mass-normalized thrust by 20 units per second, bounds integral growth to the available force and requests 50 ms run ticks. At x2 the final nominal target remains 10.26 m/s; acceleration should build gradually with no repeated launch/brake cycle. Verify on clear level ground first, then quick turns, a lower slider value, key release, jump/fall, a wall and Movelock. The options show script speed/target/state separately from viewer-measured speed: compare both to distinguish physical oscillation from display corrections; no live-grid comparison has been performed automatically. Offline evaluation uses distinct legacy/MKS masses (100 times apart), exercises cleanup and force/ramp bounds, and checks a range of deterministic braking models. This does not emulate Havok, prove smoothness on the grid or justify changing global avatar prediction. Intercepting horizontal controls is an unimplemented design candidate, not an established way to remove the simulator avatar controller.

Run authority follow-up (2026-10-15): the human measured about 7 m/s at x2 with little further gain at x5 on v1.11. Protocol 1.12 restores the Firestorm llGetMassMKS force scale while retaining the explicit 4 m/s/s target ramp, 20-unit/s upward force ramp and immediate cleanup. Verify x2 (10.26 m/s nominal target) first, then x5 (25.65 m/s) on clear level ground, both after settling and while changing the slider. The options additionally show requested thrust as a percentage of the 400-unit MKS-normalized cap, the gradual target and controller resets; a separate state indicates saturation. This is requested effort, not actual acceleration. A normal key release or sharp turn also counts as a reset; repeated resets during straight continuous running are diagnostic. Three extra bounded acknowledgement fields preserve 5/10-field compatibility; malformed powers/targets/counters are rejected. Offline evaluation now divides model force by MKS mass: with the same assumed braking constants it reproduces about 7.13 m/s at both x2/x5 for v1.11, then checks target tracking or explicit saturation across gains/braking constants, slider changes and all release paths for the actual new LSL functions. The earlier legacy-mass plant model hid this loss of authority. Neither model establishes the real Havok response. Confirm smoothness, release, jump/fall, collisions and remote animations on the grid; Movelock hold/reanchor code remains unchanged.

#### Replace a prim

To check the switch to "Edit linked parts", combine
`AURORA_DEMO_REPLACE_PRIM=1` and
`AURORA_DEMO_BUILD=linked-click,object:99688`: whole object selected, linked
editing turned on at frame 625, then click on part #9 at frame 635 (start
600 by default). Capture at 630 then at 650: 15 selected parts become a
single one, with "Link number: 9" and the replacement button.
`linked-click-ready,object:99688` stops on the whole object so that the
human can tick the option then click on the part themselves.

`AURORA_DEMO_REPLACE_PRIM=1` adds a set of 15 prims (root `99680`, card
stack at link 9: `99688`) and an independent replacement prim (`99700`). The
replacement prim has a different rotation and different dimensions; its
contents are empty. A script of the board starts stopped to check that it
stays stopped. The updates of the prims do not fill in `OwnerID`, as for a
prim without sound or particles on the grid: the permission flags and the
`ObjectProperties` replies confirm that they belong to the resident.
Choosing the replacement prim and the operation must work in this case.

`AURORA_DEMO_REPLACE_PRIM=script` keeps a single script in the card stack,
with the other contents empty. The simulated engine confirms the state
changes of the scripts only after a first reply carrying the old state. With
`AURORA_DEMO_BUILD=replace-copy,object:99688`, check the copy of the script,
the preservation of the original and the return to their active state,
without a hang. Capture the result at frame 900 to let the confirmations
arrive. The progress distinguishes reading the states, copying to the
inventory and inserting into the replacement prim; the errors reading the
contents are reported directly, with the step concerned.

Combine with `AURORA_DEMO_BUILD`:

- `replace-button,object:99688`: one linked part selected, button in the build window;
- `replace-edit,object:99688`: replacement prim chosen, five independent options, no mutation;
- `replace-run,object:99688`: replacement with the default values;
- `replace-copy,object:99688`: copy of the contents and of the size, then replacement;
- `replace-run,object:99680`: replacement of the root and check of the positions of the children.

The build scripts start at frame `AURORA_DEMO_BUILD_FRAME` (600 by default).
The replacement prim is chosen at +35 frames, the confirmation happens at
+55 for the scenarios that run the operation. The simulator sends back the
parenting changes, the contents and the states of the scripts through the
same events as the network. On the grid, also check the numbers with LSL,
the copy rights and the confirmations on a slow connection. The protocol
returns the parent of the prims, without a link number: after confirmation
of the parenting, Aurora keeps the order sent in ObjectLink, regardless of
the order in which the packets arrive. The 256 prims are attached in
confirmed groups, from the end of the list towards its start.

The button appears under the selection information of the compact inspector.
The panel uses its frame, its headings and its margins; the confirmation
actions stay outside the scrolling area.

The replacement places the old prim at the initial position of the
replacement prim, keeping its rotation and its size. With "Copy position"
enabled, the two prims therefore swap their positions. The objects must be
rezzed in the world, modifiable, belong to the resident, with no physics,
temporary status or seated avatar. Maximum: 256 prims and 1024 items
copied / scripts observed. Name conflicts and non-copyable contents stop the
operation before any unlinking. Copying a script does not keep its runtime
memory; replacing the root does not automatically copy the Linkset Data.

Scene menu customization: `customize` unlocks the object menu at frame 740;
`customize-land`, `customize-self`, `customize-avatar` and
`customize-other-attachment` open the respective editor at the same frame.
`avatar-menu` and `attachment-menu` open the locked menus. These four avatar
scenarios simulate nearby voice membership without starting an audio session:
verify the volume stays above all movable rows, including while editing, and
does not overlap Profile. Technical actions are grouped under Display,
Information, Reset, Moderation (with rights) and Disturbance. A clicked
attachment's object actions are separate from its wearer's actions.
`customize-hud` additionally needs
`AURORA_DEMO_HUDS=1`. Capture at frame 820 or later. `pbr` opens the object's
native Materials section with PBR selected. The lock button enters editing;
drag rows into groups, icon bars or the hidden shelf, right-click a row for
organization choices, and use Finish to resume normal commands. Verify that
closing/reopening relocks, each target kind retains its own layout, a different
object receives commands, and list/radial menus share visibility and ordering.
`textures` opens the read-only baked-texture grid directly at frame 740.
Show Textures on an avatar or attachment opens the same grid;
changing avatars replaces the inspected bakes without exposing full asset UUIDs.

**`AURORA_DEMO_OBJECT_MENU=list|particle|scripts|scripts-run|return|return-yes|wear|particle-block|link|drop|customize|customize-land|customize-self|customize-avatar|customize-other-attachment|avatar-menu|attachment-menu|customize-hud|pbr|textures|profile|profile-no-mod|profile-other|profile-sale|profile-hud|reset-lod|menu-export|menu-load|region-objects|characters`**
`profile` opens the bell's actual demo ObjectProperties in the directly editable
General form: name/description, group and share/deed, everyone/next-owner rights,
click action, sale type/price and search inclusion, plus actual pathfinding flags.
`profile-no-mod` revokes the effective Modify flag while retaining owner metadata
rights; `profile-other` shows another resident's object; `profile-sale` starts
with a copy for sale at L$25; `profile-hud`, with `AURORA_DEMO_HUDS=1`, opens the
HUD root without waiting for its children's properties. Names commit on Enter
or focus loss; sale drafts use Apply. Check both Profile and General remain
consistent, group names resolve, permissions disable controls as appropriate,
and selecting another object does not redirect profile edits. Open/Pay/Buy use
the explicit native object interactions; Details opens Inspect Objects.
`region-objects` and `characters` pass synthetic LLSD through the same
parser and reply correlation as the live capabilities, reveal the bell and show
its selected details. Search, select, refresh, Profile, Inspect, Zoom and Copy UUID
are available; these lists are read-only. `menu-export` opens the seven-profile
notecard naming dialog; `menu-load` opens the shared searchable COPY-aware picker.
`reset-lod` invokes our avatar's transient worn-mesh LOD reset. Capture these
windows at frame 860. On the grid, the human should save menus of several types,
alter their arrangements, import the notecard and confirm all seven restore,
locked; an inventory-menu notecard must be rejected without changing scene menus.
Verify object properties on own/other objects and attachments/HUDs, and the two
pathfinding lists in supported/unsupported regions, including a non-character
absent from the character list. Navmesh editing and terrain rows are not covered
by this read-only implementation.

At frame 600, a violet bell of ours (scripted, touch text "Ring the
bell", sit text "Lie down"), a teal crate of ours and a cloud of large
amber particles another resident owns stand in a row 5 m ahead, where the
camera looks. At frame 700 the right click opens the menu of the bell
(`list`: "Ring the bell" and "Lie down" as labels, Return enabled, the
script actions and Put on in their submenus), or lands on the particles
(`particle`: the menu of what is behind, with contextual Particles controls).
Particle clicks retain all intersected sources/owners sampled at the press while
object depth is read asynchronously. Velocity-aligned quads and ribbons use
the displayed geometry; particles against the sky expose only the Particles
submenu. At frame 740: `scripts` chooses Reset scripts (the "modify
scripts" question), `scripts-run` answers it yes (the Reset progress
window lists the bell's script, then "Done."); `return` / `return-yes`
choose Return (the question; answered yes, the bell goes); `wear` chooses
Put on ▸ Attach to ▸ Right Arm ▸ Right Hand: the avatar walks to the bell,
then attaches it (it leaves the world: the demo has no item to wear it
from); `particle-block` filters the particles' owner locally without muting
the resident. Particles exposes the clicked emitter directly and other
intersected emitting linksets under their object names, with independent
object/owner choices; unknown names resolve asynchronously from properties.
Avatar > Disturbance > Block particles shares the owner filter. Verify that
chat, voice, sounds and avatar rendering stay unchanged, toggles stay open,
and rules survive reopening/restart. `link` selects the crate and the bell in
the build tools at frame 650: the menu then has Link enabled (Object ▸)
and "Take multiple ▸" instead of Take. `drop`, with `AURORA_DEMO_WORN=1`,
opens the worn backpack's menu and chooses Drop (the question). The log
writes `demo object menu: <action>` and, at frame 900, `demo object menu
result: …` (bell in the world or gone, particle owner, filter, mute state,
remaining owner particles and queue lines). After `particle-block`, expect
`blocked=true`, `muted=false` and `live owner particles=0`.
Capture with `AURORA_FPS_LIMIT=60` at frames 760 (menus), 780
(questions), 800 (queue window, filtered particles) and 1000 (wear); a submenu
opens under `AURORA_DEMO_POINTER` (one point every 60 frames from frame
300: the eighth point is at frame 720). Our attachments' menu (favorites,
Drop, particle filters) is `AURORA_DEMO_WORN=menu`.


## HUDs

| Variable | Effect |
|---|---|
| `AURORA_DEMO_HUDS=<mode>` | Worn HUDs: display, touch, move, edit |
| `AURORA_DEMO_WORN=<mode>` | Object worn on the body: menus, Edit, Inspect, inventory double-clicks, pause while edited, temporary detach, temporary attachments in Appearance |

### Details

Eight textured HUDs on points 31–38, with floating text and a translucent
child button; another avatar's HUD is hidden. No access to a
grid.

- `1`: display only.
- `touch`: simulates press, drag and release on the child (frames
  720–900), then changes the colour of the HUD.
- `zoom`: scales down to 50%; `hidden`: hides the HUDs; `media`: shows the
  local test page on the centre HUD.
- `scale`: five wheel notches with the HUD key (frame 720), on a
  HUD without Modify / Move permissions; outline and percentage until frame
  840. `scale-ctrl`, `scale-key`, `scale-secondary` use Ctrl, B or the
  secondary key. `scale-reset` restores 100%; `scale-forget` forgets the
  profile without changing the current copy. `scale-touch` then clicks the
  enlarged HUD (frames 880–900).
  The profiles of these scenarios stay in the demo account folder.
- `scale-reattach`, `scale-rename`, `scale-copy`, `scale-temp`, `scale-temp-item`,
  `scale-temp-instance`:
  start at 100%, enlarge with the HUD key
  (frame 720), remove the whole linkset and reattach it with new object UUIDs
  (frame 900). `scale-reattach` keeps the same item without inventory creator
  metadata; `scale-rename` renames it while detached. `scale-copy` creates a
  new inventory UUID with a different inventory label but the same object name
  and creator. `scale-temp` has no inventory item or AttachItemID, like a
  temporary experience HUD; `scale-temp-item` has a foreign item UUID that
  changes between appearances and cannot be fetched from our inventory.
  `scale-temp-instance` uses the rezzed UUID itself as AttachItemID, matching
  LLViewerObject::isTempAttachment, with no per-instance inventory profile.
  The real full ObjectProperties request supplies the common name / creator.
  All six must restore
  161.1%, including the child and floating text. Profiles are isolated in
  memory for these scenarios; unit tests cover saving and restarting.
  Suggested screenshots: 710,760,920.
- `drag`: grabs the child with Ctrl (frame 720), moves the whole HUD,
  releases Ctrl before the click (frame 800), then saves the position on
  left release (frame 840) without touching any script.
- `drag-lock`: locks the HUD through its child at frame 680, attempts the
  same drag, then opens the context menu at frame 880 with the lock checked.
  The HUD must stay in place. `drag-lock-unlock` unlocks it at frame 700;
  dragging must work again. `drag-lock-camera` holds Ctrl + Alt instead:
  the camera must orbit while the locked HUD stays in place.
- `drag-zoom`: the same at 50%; `drag-ignore`: on a child with the
  IGNORE action; `drag-ctrl`: explicitly binds Ctrl; `drag-key`: the B key
  instead of Ctrl; `drag-secondary`: ALT as primary key and Ctrl as
  secondary key, moving with the latter.
- `menu`, `edit`, `edit-zoom`: right click on the child, Edit and
  moving the linkset by the Y handle, also at 50%.
- `edit-scroll`: selects a HUD locally enlarged to 200%, then
  places it off screen (frame 740). Without the HUD key, five notches
  backwards in an empty area zoom all the HUDs out (frame 760); holding the
  HUD key while clicking, moving the pointer and scrolling keeps the HUD's
  position and local scale unchanged (frame 820). The wheel still changes
  the common editing view. Five notches forwards return to the normal
  global view (frame 900). The camera and the
  local factors are kept during the global zoom. Direct held-key movement
  and local resizing stay disabled while build tools are open. The screen
  outline follows the zoomed projection. Frames 910,930,950 zoom out again
  before closing editing (920), selecting a world object (940), then
  selecting another HUD (960) and clearing the selection (970). Closing or
  leaving the HUD selection restores 100%; switching HUDs retains the zoom.
  Logs report each reset and confirm that the wheel is no longer intercepted
  outside HUD editing. Suggested screenshots: 750,790,905,925,945,965,980.
- `faces`, `faces-turned`: nine HUD panels with a single visible face —
  columns front / back / explicitly double-sided PBR material, rows
  opaque / translucent / masked. The back column must stay empty;
  `faces-turned` turns all the panels by 180°: the front column
  becomes empty, the back appears, the double-sided one stays visible. The
  floating labels stay visible both ways.
- `faces-alpha` uses a texture with holes in those panels. Left clicks
  and hover reach the whole HUD face in Blend and Mask modes, including
  fully transparent texels. Quick movement and local resizing still pass
  through texture holes; opaque PBR ignores texture alpha. A face that
  draws nothing (texture-entry alpha 0, or the library's transparent
  texture) takes left clicks and hover whatever its material mode.
  Editing and right-click selection retain access to invisible faces.
  With `AURORA_DEMO_DEBUG=alpha`, translucent/invisible HUD faces appear red
  and material masks blue; their highlighted surfaces become pickable.
- `blend-opacity`: three rows of panels, Legacy Blend, PBR Blend and Mask, at 0%, 50% and
  100% opacity from left to right. Each uses the same texture with holes.
  Ordinary left clicks through the centre hole must reach every panel,
  including the invisible Blend and Mask panels, without Ctrl+Alt+T.
  Right-click selection keeps its existing access to transparent faces.
  Combine with `AURORA_DEMO_CLICKS=<ms>,x,y,left` for repeated native touches.

Right-click a HUD and check **Lock quick movement** to prevent its held-key
drag without blocking buttons, scripts, local resizing or explicit Edit.
The lock applies to the whole linkset, survives reattachment/relogin for
ordinary inventory HUDs, and is stored per account in `hud_locks.json`.
Temporary attachments without a stable inventory item keep an instance lock.

**`AURORA_DEMO_WORN=1|menu|edit|move|freeze|inspect|inventory|library|sound|temporary|temporary-detach|temporary-drop`** — our avatar wears on its
back (Spine point) a two-prim object, touchable, modifiable, linked to an
inventory item ("Demo backpack", in the current outfit). No
access to a grid.

- `1`: worn object only, to test by hand the right click on the object,
  in the inventory and in Appearance › Current outfit.
- `temporary`: at frame 600 an experience attaches a temporary object
  ("Arena wristband") to the left hand, its AttachItemID its own object id,
  and Appearance opens on Worn: the "Temporary attachments (1)" section
  opens by itself, named by the demo's AttachmentResources answer, with
  "Worn on Left Hand". Its right click offers Touch, Edit and Detach;
  Detach sends ObjectDetach and the demo simulator kills the object. The
  log writes `demo temporary attachments` with the one found. Capture at
  frame 720.
- `temporary-outfit`: the same temporary object stays worn while ordinary
  outfit actions add an inventory attachment (frame 720), remove it (900),
  remove a jacket (1080), and put the jacket back (1260). At frames 840,
  1020, 1200 and 1380, `demo temporary outfit` logs one temporary object,
  no pending mutation or error, and attachment/clothing states of
  `true/true`, `false/true`, `false/false`, `false/true`. Capture frame 1440;
  Appearance > Worn must still show the temporary object. This exercises
  the production outfit plan, acknowledged COF update and simulator commands
  offline, without fetching the temporary task UUID as an inventory item.
- `temporary-detach` / `temporary-drop`: the backpack becomes a temporary
  attachment at frame 600, with a nil AttachItemID; its linked prim starts
  a dance. At frame 900 the simulator kills the root or unparents it,
  without an animation-stop message. The viewer must request that stop.
  The log writes `demo temporary detach` with `dancing=true` at frame 750
  and `dancing=false` at 1050; captures at those frames show the dance and
  the recovered standing pose. This uses the ordinary network-event and
  outgoing animation-command paths, entirely offline.
- `menu`: right click on the object at frame 700 (`menu_attachment_self.xml`).
- `edit`: then "Edit" at frame 740: build window, object
  outlined, arrows aligned on the attachment point, local position in
  Transform.
- `move`: like `edit`, then drags the Z arrow by 80 px (frames 780–830); the
  log writes `demo worn edit: local position=…, attachment=9`.
- `inspect`: "Inspect" at frame 740: the window lists the root and the
  linked prim, their figures and the totals; the prim of the highlighted
  row is outlined in yellow, the others in blue.
- `inventory`: the inventory opens on the item, double-clicked on its real
  row at frames 700–703 (the backpack detaches, its worn label leaves the
  row) and again at 900–903 (it is worn again: the demo server rezzes its
  two prims back under new local and object ids, as the simulator does), like the Legacy Viewer's
  double-click (LLObjectBridge::openItem). The log writes
  `demo worn inventory: double-click on the item row at frame …`. The other
  objects of the demo outfit show as worn after the first change: the demo
  lists them in the current outfit without wearing them at start. By hand,
  also double-click a worn piece of clothing (it comes off) and a worn body
  part (nothing happens: a body part is only replaced).
- `library`: the Library holds a hat, a shirt and a shape, double-clicked at
  frames 700, 800 and 900: each is copied into the inventory (the folder of
  its type, like the server's default folder) and the copy is worn
  (LLAppearanceMgr::wearItemsOnAvatar). The log writes `library copy
  arrived, wearing it: …` three times; at frame 1000 the shape's copy is
  worn in place of "Aurora silhouette".
- `sound`: a sound item playing the demo chime, double-clicked at frame
  700: it plays in the world at our position (LLSoundBridgeAction), where
  "Open" shows its preview. The log writes `demo sound trigger: …`.
- `freeze`: our avatar dances from frame 600, the object is right-clicked
  at 700 and edited at 740, and the build window closes at 1100: the
  dance stands still from the right click to the close, then goes on from
  the same pose (LLSelectMgr::pauseAssociatedAvatars). The log writes
  `demo worn freeze: frame … paused=… head=…` at frames 690, 720, 900,
  1099, 1130 and 1200: the head stays put between 720 and 1099.

The steps are counted in frames: capture with `AURORA_FPS_LIMIT=60`,
around frames 720 (menu), 770 (edit, inspect), 850 (move), 680, 800,
1000 (inventory) and 680, 760, 860, 1000 (library).

To check with these scenarios:

- Ctrl + left click held on the visible geometry moves the HUD without
  opening the tools; grab hand on hover and while moving.
- Alt and Shift+Alt do not start a HUD move. A camera drag started in the
  world continues when the pointer crosses a HUD.
- Preferences › Shortcuts › HUDs, between Windows and Debug: two keys
  configurable like the other shortcuts (Ctrl as primary by default,
  secondary empty), click to capture, right click to clear and Esc to
  cancel. To choose Ctrl, ALT or Shift alone, press then release;
  combinations and assignable mouse buttons are also accepted.
  Both choices are saved and applied on hover as well as while moving;
  "Restore default shortcuts" also resets this row.
- The Move permission is enough, including without the Modify permission.
- HUD key + wheel: local size from 10% to 800%, Shift for finer steps,
  with no camera zoom nor media scrolling. Right click: "Reset to
  100%" and "Forget saved size". New copies take
  the profile of the same object name / creator on this account, including
  temporary experience HUDs without an inventory item; copies
  already worn keep their independent sizes. Check after a restart,
  detach / reattach, clicking the buttons and moving the HUD.
  The same inventory item keeps its own size after reattachment or renaming,
  even before inventory metadata arrives. Separately resized items with the
  same name / creator must also keep their individual sizes after reattachment.
  Resize a temporary experience HUD, stand to remove it, then sit again:
  the new instance must restore the saved size. Verify after restarting too.
- Demo inventory linked to the current outfit, to test detaching and
  the display in the inventory.
- World › HUDs menu: show / hide, zoom out / zoom in and normal
  size.

## Avatars and animations

| Variable | Effect |
|---|---|
| `AURORA_DEMO_ANIM_LOOP=1\|sync\|sync-edit\|resync` | Animation loops, sequences, stops and synchronization after delayed loading, editing or Ctrl+S |
| `AURORA_DEMO_GROUNDING=slope\|steps\|landing` | Standard standing feet on uneven support and landing recovery; append `-off` for original poses |
| `AURORA_DEMO_AO=<mode>` | Offline AO manager, compact toolbar controls and movement scenarios |
| `AURORA_DEMO_ANIMESH=1` | Animated objects (animesh); `points`: a bar rigged to an attachment point the animation turns |
| `AURORA_DEMO_BLENDER_LIVE=1\|objects\|static\|static-hud\|static-attached\|static-detached\|static-linked\|static-linked-child\|static-linked-hud\|static-linked-attached\|static-linked-detached\|static-linked-menu\|static-linked-hud-menu\|upload\|lods\|shape\|shape-readonly\|shape-apply\|plugins\|installer-choose\|installer-check\|installer-install\|installer-done\|installer-error` | Local Blender bridge, static object placement, LOD import, shape transfer and installer feedback |
| `AURORA_DEMO_BLENDER_PORT=<1024..65535>` | Private loopback port for the Blender demo; use the same `--port` in the native bridge script to avoid already-open Blender clients |
| `AURORA_DEMO_ANIMESH_TRIS=<n>` | Triangles added to the demo's rigged mesh |
| `AURORA_DEMO_HAND_IK=<mode>` | Local or observed motorcycle contacts on linked targets |
| `AURORA_DEMO_IK_EDIT=<mode>` | Local pose/animation editor and target-prim contact export |
| `AURORA_DEMO_CLOUD=1` | Avatar loading clouds |
| `AURORA_DEMO_CLASSIC=1` | Classic avatars: shapes and bakes with alpha |
| `AURORA_DEMO_DEFORM=1`, `reset`, `reset-anim`, `self` or `self-reset` | An avatar deformed by a finished pose, and "Reset skeleton" |
| `AURORA_DEMO_EMOTE=<expression>[,<hand pose>][,signal][,self]` | A facial expression and a hand pose from an animation (system body) |
| `AURORA_DEMO_PHYSICS=1\|still\|off\|classic` | Avatar body physics: balls on the breasts, belly and butt bounce as the avatars hop |
| `AURORA_DEMO_DERENDER=menu\|inspect\|avatar\|hat` | A resident's Inspect, Derender and the Asset blacklist window |
| `AURORA_DEMO_SEAT_KILL=1` | A seat deleted under the resident sitting on it |
| `AURORA_DEMO_DISPLAYNAME="<name>"` | Display name change (simulated) |
| `AURORA_DEMO_DISPLAYNAME_ERROR=<error_tag>` | Server error on the name change |

### Details

`AURORA_DEMO_BLENDER_LIVE=1` opens the local bridge and places a purple shell
on our animated avatar, through the production protocol and GPU skinning.
`static` adds an editable unrigged green object on the terrain and selects it
for the standard build tools. `static-linked` adds an orange child mesh beside
the green root, sent child-first to verify order-independent parentage. The
list shows both sources and their root relationship; editing the root selects
the linked set. `static-linked-child` selects only the child with Edit linked.
`static-linked-hud`, `static-linked-attached` and `static-linked-detached`
exercise whole-family attachment or removal. `static-linked-menu` opens the
native object menu on the child, preserving whole-linkset selection;
`static-linked-hud-menu` exercises the worn local menu. Check the single root,
indented prim numbers and object/prim counts. The Edit linked checkbox belongs to
the native build tools and context menu; it is absent from the LiveMesh
window. `static-hud` attaches it at HUD Center;
`static-attached` attaches it to the right hand; `static-detached` removes its
draw object while retaining the source row. Check numeric/gizmo transforms,
body/HUD menus, detach/rez, source visibility and geometry updates, with the
rigged shell remaining on the avatar.
The listener accepts the Blender extension on port 17655. Check that live
retouches replace the shell, invalid data preserves the previous geometry,
and Stop removes all preview objects. The demo log reports `ready_skinned`
objects using our avatar's skeleton. See [LiveMesh](BLENDER_LIVE.md).

Blender's synchronization-list linksets are exercised by
`blender --background --factory-startup --python-exit-code 1 --python blender/tests/test_groups.py`.
It covers unparented multi-add, active roots, plain/Ctrl/Shift row selection,
range ordering and stable anchors, partial regrouping, whole-family root
Ungroup, selective child Ungroup across multiple families, Blender parent
overrides, root removal/promotion, paused and mixed rigged/static selections,
stable source identities, coordinate units, save/load, Undo/Redo and
automatic/manual update behavior. Check the native Blender list visually too:
click names, Ctrl/Cmd-click two separate rows, Shift-click a displayed range,
then Group only that selection. Ungroup a root alone and several children
without the root. Selection highlights, the selected count, synchronization
chain icons, root-first ordering and Group/Ungroup/Make root must remain usable
in English and French without changing Blender's 3D selection.
`objects` adds a belt and neck band, starts the belt hidden and the band
removed. Toggle the belt and clear the objects; no restore button should
appear. Each row's Import button opens that object's
ordinary mesh importer without selecting it first. Check the Connection and
Plugins tabs, and the truncated object names beside their Import
buttons. `upload` opens the importer directly with a frozen shell draft.
Connection contains status, port, shape and object controls; Plugins contains
installation and manual ZIP export. `plugins` opens the idle Plugins tab.
Check readable 16-point body/control text, including paths, hints, tabs and
manual-install labels, in EN/FR/JA and narrow windows. Switching tabs or reopening
must preserve the preview, selection and installer state. Port, address and
start/stop can wrap at narrow widths. Each tab retains its own scroll position.
Opening LiveMesh must not discover Blender installations or launch Blender probes.
In Plugins, Search for Blender explicitly detects installed executables on a worker
and rescans on subsequent clicks. Check that paths and per-version plug-in
statuses/actions are readable. Install in Blender opens an executable picker;
choosing one file checks and installs only that executable, even without a prior
search. Check the choosing/checking/installing states and spinner, then a success
message naming the Blender version and its updated row. Completion and errors
stay above the installation list, including after a search with many results.
Existing rows must remain.
Cancel must leave the list alone without an empty-search error. Unsupported
versions, newer installed plug-ins and failures must show an error, not success.
Manual ZIP export remains collapsed. A fresh Blender scene defaults to a 2-second
automatic update interval; intervals already saved in `.blend` files are retained.
Detection alone must not install or enable anything.
`installer-choose`, `installer-check` and `installer-install` hold synthetic
installer steps with their spinner and disabled controls. `installer-done`
shows a successful Blender 5.2.1 installation and `installer-error` a failure.
These fixtures never open the file picker, discover executables or change a
Blender profile. Capture the steps and completion in English and French.
`shape` seeds a synthetic editable shape and sends it once a capable Blender
client connects; `shape-readonly` removes Modify and must send nothing. Neither
mode uses account assets or a grid. Run `blender/tests/bridge_shape.py` with
`-- --port <port>` against the offline viewer; it checks a native import and
streams a reference part back. Add `--readonly` to verify no shape arrives.
`shape-apply` sends the same synthetic shape as an Apply action; run the bridge
with `--apply` to create/select a test rig, receive the shape on that same rig
and return its weighted shirt. Connection starts with an empty sync list.
Pass `--hold 45` to keep Blender connected while capturing the viewer's controls.
Native shape tests accept a synthetic fixture: set `AURORA_TEST_SHAPE_OUTPUT`
to its JSON path, run `cargo test --workspace blender_live_shape_payload`, then
run Blender with `--background --factory-startup --python-exit-code 1 --python
blender/tests/test_shape_transfer.py -- <fixture.json>`. Precreate isolated
Blender user directories and use a private temporary directory for UI tests.
No fee or upload request is sent automatically. Check skin weights, LODs,
physics, the source snapshot notice and the separate-PBR-upload notice.

`lods` waits for a real local client and requests its first source's prepared
LOD bundle once. Run `blender/tests/bridge_lods.py -- --port <port> --hold 45`
using Blender's isolated background arguments. It supplies a hidden, named
Medium mesh and automatically reduces Low and Lowest using distinct budgets.
The normal importer must open with four File levels, decreasing triangle counts
and retained skin/UV/materials. Select each LOD and test the bend slider in the
skinned preview. No cost request or upload is sent.
`blender/tests/test_lods.py` covers manual priority, name matching, automatic
budgets, hidden meshes, weight/material validation, original-data preservation
and `.blend` persistence. An optional `-- <fixture.json>` saves its native
four-level fixture; set `AURORA_TEST_LODS_OUTPUT` to that path and run
`cargo test --workspace blender_lods_native_fixture -- --ignored --nocapture`
to check the same bytes in Aurora's decoder and aligned scenes.

The installer bridge can be exercised without a grid using system Python:
`python blender/tests/test_installer_bridge.py <blender-executable> --extension-zip <extension.zip> --legacy-zip <legacy.zip> --work <new-temporary-directory>`.
It creates every isolated user directory **before** launching Blender (older
Blender can ignore a nonexistent configuration override when saving). It verifies
fresh installation, upgrade, persistent activation, reactivation, backups,
rollback after broken packages, downgrade refusal, classic-to-extension migration
and preservation of an unrelated add-on/preference. It never saves a scene.
To exercise Rust's actual bundled package/process path too, set
`AURORA_TEST_BLENDER` to the executable and run
`cargo test --workspace blender_live_installer_native -- --ignored --nocapture`.
That ignored test creates and removes its own isolated profile.
- **`AURORA_DEMO_HAND_IK=1|moving|release`**: at frame 300 seat our avatar on
  a synthetic owned vehicle with two coloured wrist anchors and a riding
  animation. The normal nonce/chat handshake installs position-only palm contacts,
  preserving each animated wrist's articulation. `moving`
  tilts and turns the vehicle continuously; `release` stands the rider at
  frame 720. Capture frame 620 for contact and frame 800 for release. Pure
  solver/protocol and scene regressions cover live arm-length changes,
  mesh joint offsets, hand-size and nonuniform hand scales, palm and wrist
  points, nonstretching unreachable goals, authored orientations
  and position-only contacts preserving animated wrist articulation,
  tilted parent transforms, stale messages and contact expiry. The human
  installs the [LSL script](../scripts/lsl/motorcycle-hand-ik.lsl) on their
  motorcycle and validates real compilation, steering and region crossing;
  see [HAND_CONTACTS.md](HAND_CONTACTS.md).
  `observer` seats a separate avatar on a moving vehicle and answers the
  protocol-5 handshake; our standing avatar stays untouched. `observer-release`
  stands that rider at frame 720 without an explicit release packet. Capture
  620 for remote contact and 800 for release; the demo logs active remote/local
  graph counts. These fixtures do not connect to a grid.
  `objects` and `observer-objects` use a source-local left link and an independent
  right target object selected with `_ROOT_`. Both move through ordinary object
  updates under one rig declaration. Capture 620 to inspect local/remote tracking
  after finite name-property queries, without a second publisher or grouped targets.
  Startup regressions resolve a cold three-prim linkset on consecutive simulated
  frames, retain unique-name validation and verify the shared rolling limit of
  four queries per second even across source retirement/redeclaration. Packed
  UTF-8 declarations traverse the authenticated worn relay for local/remote
  avatars and commit only after all fragments arrive, including reverse order.
  The companion's packet packing retains its timer cadence and relay byte limits.
  Child-name regressions model family replies containing only the root: the
  old path stalls at one of six names after sixteen attempts, for both seated
  and worn sources. Individual select/immediate-deselect reads resolve all six
  while retaining the same finite query budget. Selection tests preserve both
  whole-linkset and linked-part Edit selections, never queue transforms and
  query roots individually too, including when family responses are entirely
  absent. The child-only path then stalls at five of six names after eight
  attempts; all-individual reads complete without weakening uniqueness checks.
  J8 identifies unknown root/child roles without object IDs. The motorcycle DEMO answers child names
  through ObjectSelect rather than unrealistically returning them to family reads.
  Reception regressions send off before/during the initial definition and verify
  finite recovery for local/observed worn and seated sources. Replacement tests
  reject acknowledgements from late old-bundle state and an early off, then
  activate the completed new definition and remain quiet afterward.
  J8 regressions preserve the last decoder/envelope failure across later off
  events, report accepted activations and editor suppression without private IDs
  or network side effects, and distinguish waiting/resolved/duplicate links.
  Combine `AURORA_DEMO_TOOLBOX=0:J8` with an IK runtime fixture for its native
  report; repeat with the editor open to inspect suppression flags.
  `profiles` and `observer-profiles` declare cached `drive` and `gas_only`
  keywords and pulse the right anchor while playing ordinary animations.
  Frame 480 selects the right hand alone; frame 600 restores both hands without
  changing animation. Frame 720 releases while still seated, frame 840
  restarts, and frame 960 simulates a release/restart missed by the viewer.
  Capture 560, 660, 800 and 1020 for one hand, both, release and restart. The
  observer stays standing; logs include active graph and binding counts.
  The runtime-limit regressions also cover unused-seat/unworn announcements,
  unsolicited protocol floods without chat retention, fresh use after discard,
  full/inactive/partial/editor-snapshot expiry, exact 8192-byte boundaries and
  cumulative caps across 25 live attachment publishers on five avatars. Inspect
  `observer-release` at 800 after standing and `observer-profiles` at 560/660/800/1020
  to verify those admission/lifetime limits preserve normal contact selection.
  `ownerless` and `observer-ownerless` clear the vehicle's ObjectUpdate OwnerID,
  like full updates without sound/particles, while retaining authenticated chat
  source/owner metadata. Capture 620: contacts must still apply to the driver
  or observed rider. A simulator chat with a missing owner or known ownership
  conflict remains rejected; worn sources require the chat owner to be their actual wearer.
- **`AURORA_DEMO_IK_EDIT=poser|poser-face|poser-hands`**: open the standalone
  local poser at frame 420 and turn the head at frame 450. The face/hand
  variants select a corresponding Bento joint. Capture 620: grouped bone
  list, parent-local numeric controls, markers and native save/load/import
  actions. No motorcycle, contacts or IK-project draft is loaded or written.
  Prefer the dedicated `AURORA_DEMO_POSER` scenarios below. See [Poser](POSER.md).
- **`AURORA_DEMO_POSER=self|connected|camera|scale|hands|face|misc|physics|library|settings|close|square-head|animesh|consent-accept|consent-refuse|consent-timeout|consent-withdraw|consent-request|consent-reminder|bridge-starting|bridge-silent|bridge-failed|bridge-off`**:
  the full handoff trial, with the real local pose layer and consent state
  machine, on a standing avatar without motorcycle contacts or bone bindings.
  The additional `connected` mode restores a T-pose at 450 and moves the
  left wrist down/inwards at 480; capture 620 shows the bent elbow and
  connected wrist, with the position inspector in avatar coordinates.
  `camera` views the same head rotation from a quarter-turn camera orbit;
  compare the trackball axes with `self`. `scale` shows the head's local
  scale handles; `physics` shows the collision volume scale handles. All
  body-part scenarios display only the selected tab's joints and shafts.
  Open at frame 420, edit at 450, capture 620. Animesh selects an
  owned synthetic control skeleton. Consent fixtures request at 450, answer
  or expire at 480, and optionally withdraw at 560. Incoming-request and
  reminder modes show the target UI; accept/withdraw modes select the peer
  and edit its head at 510. `close` asks to close the window at 700, the
  head edited: capture 760 for the confirmation (Minimize, Close anyway,
  Don't ask again). `square-head` sets our Head Shape slider to 100 at frame
  300, which mirrors the forehead joints' scale on one axis (#1249): capture
  620 must show the pose on (status "Posing", Stop button) and the head
  turned, as in `self`. Library widens the native window; settings and
  the remaining modes select the corresponding controls. The other modes
  count as a ready bridge; `bridge-*` modes open the subject menu at 440 with
  the peer's Ask disabled and the matching hint (starting, query script
  silent, failed, taken off), plus Repair for `silent` and `failed`. Synthetic events
  do not validate real LSL delivery: two human-controlled grid clients must
  check bridge 1.34 provisioning and the complete consent round trip.
- **`AURORA_DEMO_IK_EDIT=1|move|release|gizmo|gizmo-local|gizmo-rotate`**: reuse the synthetic motorcycle,
  open IK Edit at frame 420, select the left wrist's palm point and the left
  linked target. `move` changes palm position and enables orientation;
  `release` closes the editor at frame 720, restoring the motorcycle's
  original contacts while the rider remains seated. Capture 620 for the
  visible skeleton, fields and offsets; capture 800 for restored playback.
  `links-add` shows the highlighted source and direct Bind to a prim/bone actions;
  `links-bone-pick` waits for a skeleton target while holding the left-wrist
  source; `links-bone` saves an own-head target, `links-prim` saves Left, and
  `links-visible` opens bone groups and the searchable list. Capture 620.
  `links-menu` opens the row pencil's Prim/Bone dropdown; `links-pose`
  opens the selected-source details in Bindings. Options occupy a shared fixed
  bottom panel with the two export buttons in every IK tab, outside body scrolling.
  `library` shows the account-local project controls inside Projects with a
  motorcycle project and its profile set; `library-name` shows inline project
  duplication naming.
  `library-projects` opens the project list with left-aligned names and
  right-aligned rename/duplicate/file-export/notecard/delete icons on every row;
  `library-delete` opens its short
  local-deletion confirmation. Inspect the shared tab strip/footer, selected
  row, filtering and inline naming at the default window size. Clicking a name
  opens Bindings; row actions target that project without switching the working
  pose. File export sits between duplication and notecard export, with no
  standalone export button above the list. Save and cancel exports of both
  the working project and another row; check all profiles/animation frames and
  preserved unsaved edits. Other tabs show only the centered read-only current project name above the strip; project IO, target
  selection and import/recovery controls must not appear in their bodies/footer.
  `library-target`, `library-target-pick` and `library-target-missing` show
  object designation, the scene-click hint and an unavailable source. Inspect
  the project target row and all profiles after assigning a replacement.
  `library-menu`, `library-object`, `library-object-import`,
  `library-object-waiting` and `library-notecard` cover the unified creation
  menu, object selection, successful fresh-project import, pending transfer
  and inventory picker. Imports preserve the current project before switching.
  `library-empty`, `library-new`, `library-profile` and `library-profile-name`
  show explicit initial creation and proposed project/profile names, without
  implicit entries. `profiles` shows named profile controls with independent smoothing and
  bindings; `profiles-animations` shows the selected profile’s ordered base
  animation stack with grip handles and numeric frames.
  `links-multiple` shows two independently calibrated associations of the same
  wrist to Left/Right, with 40/60% weights and the second row expanded.
  `links-search` retains the same two contacts while filtering the list to Right.
  Check full source/target names, influence and right-aligned secondary actions,
  a single selected contact fold, and the collapsed IK chain. Search must never
  alter the pose, activation, profile membership or either export. Clipboard
  actions briefly show Copied/a check without moving the shared footer.
  `links-offsets` selects the right wrist bound to Right, opens Offsets and contact
  and scrolls the inspector into view. Capture 620: Pose, target offset and contact
  calibration have separate frame captions and aligned XYZ tables. Check metre/degree
  labels, compact root/Detach and distance/reset rows. `build-moved` also shows the
  edited-prim shortcut alongside the two bind actions. Drag target-offset numbers:
  only the chosen component changes; calibration, root, influence and timing remain intact.
  Check the script-IK mask, four limb activations, Symmetry, World/Local and Reset pose
  in `rig-bindings`, `graph-keys`, `animations` and `links-visible`; the header's
  import icon exposes active script-contact import. These controls are disabled
  during target designation. `shortcut` dispatches Ctrl+K through the configurable
  command table at frames 420 and 520; capture 620 and check both logged opens.
  On the human viewer, Ctrl+K opens/restores the panel without discarding current
  edits; the focused script editor keeps its line-comment command. Position/rotation fields belong to the selected-source card. Translation arrows/planes appear by default, with no Gizmo
  switch. Shift temporarily hides them for free point dragging; Ctrl shows
  rotation rings and takes precedence over Shift. An active gizmo drag keeps
  its original operation if modifiers change. Verify real/virtual bones,
  limb grips, bend targets and the native animation creator.
  Check both direct actions, the row pencil's Prim/Bone choices, preservation on Escape,
  immediate binding after a scene click, and no other-avatar chooser.
  Standard Aurora IK tabs expose Bindings, Virtual bones, Animations and
  Display. The editor opens with empty authored bindings and masks
  received script IK by default; unmasking/closing restores current runtime
  contacts without stopping scripts. Explicit import keeps authored contacts.
  `seat-reload`, `seat-reload-off` and `seat-reload-waiting` request a fresh
  seated-object definition at frame 460 through normal app dispatch. Capture 620:
  the first two show two applied wrist bindings, with the inactive-phase notice
  in `off`; `waiting` keeps the original draft and displays the waiting status.
  Check the always-visible reload button above the selected-source card,
  disabled while pending or standing. Human grid test: reload on the motorcycle,
  verify fresh offsets and virtual parents, move/rotate a target, retry a silent
  publisher, stand during a request, and reload during Idle using a current export.
  Rows show activation, source bone, named target, pencil, strength, Follow rotation
  and trailing delete. The rotation-arrow button stays lit when following orientation;
  toggling it must preserve calibration, selection and inactive state. Click the target prim name: object Edit opens on that exact child
  while the IK source stays selected. Unknown names are fetched read-only
  without changing Edit selection and never displayed as UUIDs in these rows.
  Combine with `AURORA_DEMO_OPTIONS=0` to cover the skeleton with Preferences:
  bones and handles must stay behind the window. The pointer regression
  checks translation, Ctrl rotation with a fixed contact and covered clicks.
  Translation arrows/planes must appear without a modifier. Hold Shift to
  drag a point freely, or Ctrl for rotation rings. A grabbed ring must keep rotating after releasing
  Ctrl, until mouse-up. Repeat on a virtual bone. Bindings holds Detach, chain
  root and bone reset in its source card, and world/local space and pose reset
  in Options. IK Edit has no separate Pose tab. Contact calibration and relative
  target offsets are in an advanced selected-source section. Detach updates the binding mode without recapturing its
  offset. Native clip authoring retains its chain/space/reset controls in Pose.
  `gizmo` shows world-axis arrows/planes, `gizmo-local` uses the wrist's axes,
  and `gizmo-rotate` shows world-axis rotation rings. Both complete-script and
  ready-to-paste `string RIG = "…";` exports stay at the bottom, including
  with an empty or short list. The embedded JSON includes only bound bones and virtual definitions,
  matches the rig script's RIG bytes, and retains disabled/orientation/influence state.
  The complete script includes
  saved associations, offsets, modes, weights and enable states.
  `rig-bindings` saves four bone/prim associations. `rig-activation` shows
  the always-visible Global smoothing controls with 0.6 s entry and 0.8 s
  release, and the selected binding's visible 0.5 m attachment distance.
  There is no viewer activation-delay control or exported delay field.
  Pose editing stays immediate; both exports must retain those three values. Runtime regressions compare a
  restarted two-hand solve against a new pose, including off/on between frames,
  immediate proximity entry, discarded obsolete delays, independent fades, zero durations, moving
  source animations during release and authenticated cached epoch changes.
  For periodic renewal continuity, capture `AURORA_DEMO_HAND_IK=moving` and
  `observer` at frame 6600 (about 55 seconds at 120 FPS). The synthetic
  `demo IK continuity` samples at frames 600, 3600 and 6500 must retain the
  same source session, epoch, bundle and binding generations across two real
  25-second requests. The hands keep following the moving targets. Regressions
  also cover four renewals each for drivers, observers and worn sources,
  cached replies, full reloads and rejection of stale request nonces.
  On the grid, replace the complete companion once for its epoch header, then
  test repeated ready/drive/idle cycles without standing and a restart during
  release. `rig-weight` uses 50%
  influence. `rig-elbow` / `rig-knee` select a bend diamond: rotation rings
  must be centred on the actual elbow/knee. Bend diamonds stay at their real
  bend targets. The hand/foot grips and manipulation axes stay directly on
  their contacts. Clicking a grip must not move the target; dragging preserves the
  initial pointer offset. Bone-target picking stays on the skeleton itself.
  Only enabled limbs show IK effector/bend markers. Manipulating a bend control keeps its hand/foot
  target and orientation fixed. `rig-disabled` switches all four off at frame 520; capture
  620 to inspect their independent checkboxes, stored targets and release.
  `rig-body` advances both hands beyond the local arm reach: the torso/chest
  must participate while both foot contacts remain active. The four activation
  switches create extended roots (torso for hands, pelvis for feet); numeric
  roots remain editable and are exported for runtime use. Bend controls must
  remain available on extended chains. Segment lengths and IK translations
  remain unchanged; partial weights must not accumulate across reconciliation.
  `rig-released` disables all four at frame 520, then translates/rotates the
  right ankle at 560. Its association becomes active in parented mode, with
  Detach unchecked and its own limb IK switch on. Selecting/refreshing a disabled
  limb without editing must not reactivate it; turning IK on again restores
  its extended root. `torso` moves/rotates mTorso with Detach unchecked and
  segment lengths preserved. `human` opens
  Display with the default Human preset: no wings, tail, hind legs or duplicate
  mSpine pivots. Torso/chest lines connect to the visible human ancestors.
  Extended spine remains available as its own group or through All.
  `pose-hierarchy` rotates the head before the neck, then the left collar,
  shoulder, foot and toe. The neck rotation rings are shown at frame 620;
  `pose-toes` shows the toe rings after the same sequence. Children retain
  their local edits and follow their parent, without ancestor jumps or new
  limb pins. Pure regressions repeat large rotations on both arm/foot sides,
  retain a child edit while rotating its parent, check repeated-frame stability,
  keep a solved manual move local, and preserve explicit prim bindings and
  pelvis/shape offsets.
  `segments` highlights the upper-arm shaft belonging to mShoulderLeft;
  `segments-toe` highlights the terminal mToeLeft tip with the gizmo off.
  Click a shaft away from either endpoint: upper arm selects shoulder and
  forearm selects elbow. Hover shows the shaft name. A virtual parent connector
  selects its node. Point/effector/pole/gizmo hits remain prioritary; Alt,
  hidden display groups, windows, prim designation and object handles must
  suppress competing segment selection. Unit/pointer regressions cover narrow
  diagonal hits, depth ties, shoulder/forearm/neck/foot clicks, endpoint
  priority, locked-source bone-target designation and canonical leaf tips.
  `rig-groups` hides every set except arms; only arm segments/handles/poles
  remain, while the four IK goals keep applying. All/None/Human and individual
  set labels change display only.
  `animations` shows two inventory-style layers at different frames;
  `animations-reordered` reverses equal-priority order, `animations-frame`
  changes the first sample and disables the second, and `animations-neutral`
  excludes live inputs with both layers disabled. Capture 620 to inspect
  the Animations tab, independent frame sliders/numeric fields and export
  footer. `animations-release` closes at 720; capture 800 for resumed contacts.
  These samples neither upload animations nor send animation-stop messages.
  `clip` opens native animation creation with head/pelvis keys at 0/30/60,
  displayed at frame 15. `clip-frame` shows 45, `clip-priority` shows per-bone
  priority/track switches, `clip-settings` shows native timing, and `clip-crop`
  trims to 15–45. `clip-import` stages native bytes in the ordinary inspector
  without uploading. `clip-release` closes at 720 (capture 800 for restoration).
  See [ANIMATION_CREATION.md](ANIMATION_CREATION.md).
  `graph` opens a parented procedural virtual control; the virtual list pairs
  each name with its parent. `graph-empty` opens the same tab without virtual
  controls; Add control must directly follow the tabs, with no instruction text.
  There are no move/rotate/world/local buttons in this tab. `graph-keys` shows
  keyed ping-pong with a Bézier curve, `graph-remote` targets another demo
  avatar's head, and `graph-cycle` deliberately makes the controlled wrist
  depend on itself: it must release and show a cycle warning.
  `rig` enables four hand/ankle goals with explicit elbow/knee poles;
  `rig-poles` swivels all four bend planes by 0.65 radians while keeping
  effector positions fixed; `rig-symmetry` mirrors a left-hand adjustment.
  The four large effectors and four diamond controls are labelled separately.
  Pure regressions cover general chains, all named bones in direct mode,
  source validation, snapshot restoration and target-space round trips.
  Human tests cover pointer dragging, target picking, clipboard exports,
  actual worn meshes/shapes and LSL compilation. See [IK_EDIT.md](IK_EDIT.md).

`AURORA_DEMO_GROUNDING` places our avatar on a solid ramp (`slope`) or
different-height steps (`steps`). `landing` starts with a synthetic standing-up
clip on the ramp. The fixture hides the system skirt to expose both knees
and ankles. Append `-legacy` (or `-off`) to select automatic native
motion placement instead of Aurora. The native pass uses the emulated
simulator foot plane and terrain only: on separate steps it follows the common
plane, while Aurora follows each exact support. Compare `slope-front-legacy`
and `slope-front`, then `steps-front-legacy` and `steps-front`.
Append `-ao` to publish the same standing clip under a custom AO identity: native
placement must leave it alone, while Aurora can still adapt its contacts.
`landing-front-legacy` retains the authored leg bend and adapts the pelvis;
`landing-front` uses Aurora's independent terrain correction.
`-legacy-transition` starts in native mode, enables Aurora at frames 660/720,
and returns to native at 666/750; ordinary `-transition` runs the reverse.
Check the first toggle frame and frames after 0.5 s for continuity and a full
return. Explicit contacts and IK Edit must keep control in either mode.
The “Also while moving” preference defaults off. Append `-moving` to enable
it in a grounding demo; compare `steps-front-motion` with
`steps-front-motion-moving` at the same frames. Movement releases placement
over 0.5 s when off and restores it at rest; an in-place turn keeps support.
For native standing Movelock, combine `steps-front` with
`AURORA_DEMO_MOVELOCK=native`: verify standing correction after the confirmed
anchor. The terrain fixture preserves that confirmed sit when it installs the
steps. Walk and stop on the supports to check release and rearming; adding
`-motion` also exercises release during scripted movement. Manual ground sits
and chairs must never acquire standing placement.

The avatar is placed only once: walk, strafe, turn, crouch, fly or jump with
the normal controls, then stop on either support. Jumps
trigger recovery on actual return to the surface; walking uses the demo's
normal gait clips. Append `-off` to compare native motion placement, or
`-motion` for an automated side-step/stop/return/jump input sequence. Compare
`slope-lift` with `slope-lift-off` (or `steps-lift` with `steps-lift-off`):
the stand alternately raises each foot.
The raised leg must keep its authored clearance over the terrain, while the other foot
keeps its support. If a swing foot intersects a step, only the necessary
upward correction should prevent penetration; it must not be forced down to
the floor. `steps-front-walk` and `steps-reach-front-walk` use an independently
authored walk with explicit stance/flight paths; `-off` selects native standing correction for this synthetic standing clip.
On flat support, adaptation should retain that source animation. During real
movement across a step, inspect the rendered body rather than its correction
offset: a sudden change of the physical centre must not pop the body upward.
Heel/toe rolls should survive the slope correction. `steps-reach`
starts with the physical root on the high step and a foot over the low step:
the pelvis must compensate enough for both soles to reach their surfaces,
without stretching the legs. Compare `steps-reach-off`, then walk across the
seam and jump with the normal controls. In `steps-reach-lift`, the pelvis must
stay prepared for the lower foot's return rather than popping up and down
with each swing. `steps-rise-front` places both soles on raised supports over
a narrow groove beneath the physical centre. The body must rise smoothly
instead of leaving both knees compressed; compare `steps-rise-front-off`.
Append `-transition` to interrupt a fade-out after 100 ms, then fully disable
and re-enable placement (frames 660/666 and 720/750 at the 60 fps cap). Both
returns should continue from the visible pose. Terrain offsets settle toward
lower support in about 500 ms, while source foot lifts remain immediate.
Collision correction takes priority over smoothing within the configured
correction-height limit.
`steps-range-reach-front` uses a 0.9 m upper support: the saved correction
limit starts at 0.4 m, then changes to 1 m at frame 900. Compare frames 850
and 950, and inspect the live slider in Preferences › Aurora › Animations
(`AURORA_DEMO_OPTIONS=avatars/3`). Feet and body must respect the limit;
supports beyond it may remain out of reach. Ordinary modes use 0.4 m.
`steps-rise-front-lift` must retain this height while either foot lifts and
replants. An intentional crouch (normal downward control) must retain its
authored flexion. Inspect
from the front with `steps-reach-front` and `steps-reach-front-lift` (the
fixture moves west to keep the frontal camera outside the plaza deck): each
knee must bend forward on its own side, including when either step is higher,
and must not turn a nearly straight leg inward across the other knee. Inspect
each foot's height and orientation, knee bending, and the recovery's return
upright. Toggle Preferences › Avatars › Animations › Use Aurora foot placement
without restarting: corrections enter and leave smoothly, and should behave
alike at 30, 60 and 144 fps. Imported animation previews and higher-priority AO poses
of priority 5/6 must retain control. These fixtures never connect to a grid.

AO quick controls: `AURORA_DEMO_AO=quick` opens the compact panel above the
bottom AO button with a complete swimming set. `quick-missing` and
`quick-empty` show the disabled swimming gate and empty catalogue. Inspect
English/French, the set selector (selection keeps the panel open), the
previous / auto cycle / next buttons beside it, AO enable, mutually exclusive
swimming/water walking and the manager button. Check Escape, click-away and repeated toolbar clicks.
Existing movement preferences and the full manager must reflect the same
choices. The other menus/commands still open the full manager directly.

`interface` opens General; `interface-sitting` selects Sitting in the same
category. `interface-running` opens Ground with Running selected;
`interface-jumping`, `interface-flight` and `interface-swim` open the other
categories. Their order is General, Ground, Jumping, Flying, Swimming: Flying
is penultimate and Swimming last. General lists Standing, Sitting, Sitting on
ground, Standing up, Typing and Always, with sitting immediately after Standing.
Ground orders Walking, Turn right, Turn left, Running, Crouching and Crouch walking. Jumping orders preparation,
jump, fall, soft landing and hard landing, with Jumping selected on its first
visit. Flying orders Hovering, Flying slow, Flying, Descend and Ascend.
In Swimming, Underwater floating immediately precedes Underwater swimming, Obstacle exit follows Flight exit, and Cushioned water entry follows Cannonball.
Switch categories, return to a previous state and
use Up/Down in the state rail; all original states remain accessible, including
Typing and Always under General. Animation lists/groups/tracks keep their existing edits.

AO readiness and legacy swim migration validation (2026-10-18): full `scripts/check.ps1` green, 3,569 passed, zero failures, 26 ignored; scripts, formatting and Clippy clean. Regression tests check actual rail marker placement and disappearance of the missing warning, optional underwater swimming, alias fetching, preserved original targets/cycles and read-only legacy sources. Built the default dev viewer. Inspected `.scratch/ao-readiness-{ready-fr,missing-fr,ground-fr,missing-en,quick-fr,movement-fr,migrate-fr,migrated-fr}.png`; the English incomplete-set capture uses 1024x768, the others 1440x900. Shared warning glyphs and text fit, optional states are unmarked, Ground order is correct, and manual migration acknowledges a new selected set while retaining the native default. The previous `.scratch/ao-readiness-before-fr.png` provides the comparison. Last-five profile medians: before/complete/missing FR all 60 FPS, UI 1.09/0.71/0.72 ms and GPU 1.91/2.01/2.67 ms; scene motion and shared desktop/compilation load differ, so these are regression observations, not an isolated speed comparison. Scoped logs have zero errors/panics and only the same baseline synthetic missing-outfit warning. Human acceptance of the native UI and a real inventory migration remains required before PR. Suggested Aurora-only fallback mappings are a proposal, not automatic assignments.

Swimming activation and approved migration completion (2026-10-18): the gate checks eight populated states in the selected, fetched native set. Each state needs at least one resolved inventory animation in a main entry, simultaneous group or independent track; eight distinct assets are not required. The gate remains global even when the user does not use the exit transitions. An incomplete selection disables swimming. Active swimming owns its AO contexts even when ordinary AO overrides are off or paused.

| Required state | Movement context |
|---|---|
| Float at the surface | Surface idle |
| Swimming at the surface | Horizontal movement at the surface |
| Float underwater | Underwater idle |
| Swimming up | Underwater ascent or the return-to-surface action |
| Swimming down | Underwater descent or the surface dive action |
| Leap into flight | The near-surface exit into flight, normally double ascent |
| Leap onto an obstacle | The physical shore/obstacle jump, normally one surface ascent press |
| Cushioned water entry | The initial downward entry fall while buoyancy cushions it |

Swimming underwater, Dive jump, Dive entry and Cannonball are optional. An empty underwater swimming list reuses the surface swimming list at runtime. During both explicit migration and initial legacy reconstruction, the new native set receives these approved copies when the donor is populated:

| Aurora target | Existing donor |
|---|---|
| Float underwater | Float at the surface |
| Swimming underwater | Swimming at the surface |
| Leap into flight | Flying up |
| Leap onto an obstacle | Jumping |
| Dive jump | Flying |
| Dive entry | Swimming down |
| Cannonball | Crouching |
| Cushioned water entry | Falling |

Copies preserve main lists, groups and independent track cycles through new native folder/link identities. Existing populated targets win, absent/unresolved donors leave explicit gaps, and the four original legacy swimming slots are never guessed from ordinary flight. No asset upload, source edit, automatic swimming enable or repair of an already existing native set occurs. The resulting fields are independent and remain editable; Cannonball/Crouching is a provisional pose until a dedicated animation is assigned. In `migrate-swim`, inspect the copied Underwater floating list, other water mappings, readiness and original native default after acknowledgement. Unit and integration tests cover exact mappings, idempotence, authored targets, track-only donors, unchanged source originals and a missing Flying up donor keeping the gate at seven of eight.

Approved AO completion validation (2026-10-18): full `scripts/check.ps1` green, 3,573 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. Built the default dev viewer and inspected `.scratch/ao-completion-migrate-fr.png`, `ao-completion-swim-fr.png` and `ao-completion-swim-en.png` (EN at 1024x768), compared with `ao-completion-before-fr.png`. The acknowledged migrated selection shows the copied Underwater floating list, required green checks and the complete-state message; the new migration help fits. Exact mappings, cycle/group/track preservation, source immutability, authored-target precedence, idempotence, both migration paths and the missing-donor activation guard pass offline. Last-five profile medians for before/migration/completed FR: 60 FPS throughout, UI 0.68/0.66/0.66 ms and GPU 1.65/2.27/2.47 ms; scene motion, background compilation and desktop load vary, so this is not an isolated speed comparison. Owned logs show zero errors/panics and the same single synthetic missing-outfit warning as the baseline. Human validation of a real source set remains required before PR.

Water control help (2026-10-18): Movement shows one short gesture per row for surface descent, underwater ascent, surface shore jump, double-tap flight exit, jump/descend dive, cannonball and leaving water walking. Keycaps read both current Up/Down bindings, including modifiers, keyboard layout and mouse buttons; no default Page key is assumed. Unassigned actions are labelled explicitly, and the detailed dive tooltip interpolates the current descent bindings. The dense water/AO paragraphs and the swimming-animation navigation button are removed. `movement` previews default keys; `movement-rebound` supplies offline F10 / Alt+mouse-back ascent and F11 descent, with no secondary descent. Compare FR/EN at 1440x900 and EN at 1024x768: alternatives, double-tap and sequential gestures must fit, and rebinding must update every row immediately. Test secondary-only and unbound controls too. The native regression reads the actual rendered keycap labels before/after changing keyboard, modifiers, mouse and unassigned slots. These explicit demo bindings never alter a real user's settings; movement input and physics remain unchanged.

Water controls validation (2026-10-18): final `scripts/check.ps1` green, 3,573 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. The rendered regression updates default Up/Down controls to modified keyboard and secondary mouse bindings, then to a new primary binding and unassigned descent, checking that stale keys disappear immediately. Built the default dev viewer. Inspected `.scratch/ao-water-controls-compact-fr.png`, `ao-water-controls-rebound-compact-fr.png`, `ao-water-controls-compact-en.png` and `ao-water-controls-compact-small-en.png` (last at 1024x768, others 1440x900), compared with `ao-water-controls-before-fr.png`. Read-only hint rows use body text height, not button height: all water controls fit the normal window, and the smaller viewport keeps ordinary vertical scrolling. Double-tap and sequential gestures, modifiers, alternatives and mouse buttons remain readable; the old animation-navigation button and AO implementation paragraph are gone. Last-five medians for before/default/rebound FR: 60 FPS throughout, UI 0.59/0.65/0.63 ms and GPU 1.66/1.65/1.65 ms; shared desktop load and scene motion vary, so this is a regression observation, not an isolated speed comparison. Owned logs have zero errors/panics and only the same single synthetic missing-outfit warning as the baseline. Human acceptance remains required before PR.

Passage / water-walk layout: in Movement, obstacle passage occupies the left column and water walking the right, directly below the run/flight boosts. Passage has a short explanation and the current shortcut; its result feedback remains full width below the columns. Water walking keeps its current descent keycaps and disables swimming when enabled. Check both water toggles, default/rebound keys, FR/EN and a 1024x768 viewport; swimming and dive controls must remain accessible below the paired settings.

Movement presentation follow-up (2026-10-19): the passage column reserves two theme-spacing units inside its right edge, so wrapped prose stays clear of water-walk controls without shifting their alignment with flight boosts. Passage and the dive tooltip use the same keycaps as water controls, with current primary/secondary, modified, mouse and unassigned bindings; the old inline passage shortcut string is removed. Surface hints are consecutive (submerge, leap, double ascent for flight), followed by underwater ascent. Remove "automatically" from the shore-jump label in EN/FR; input and behavior remain unchanged. The explicit offline `movement-rebound` fixture now requests a 520-pixel manager width. Inspect default/rebound FR and narrow EN layouts, including long custom chords.

Optional water controls (2026-10-19): built-in and restored toolbar defaults omit swimming (`movement`) and water walking (`water_walk`). Both remain in the customization catalogue and AO quick controls. Login no longer inserts water walking into existing customized bars; saved placements remain authoritative, and the retired upgrade flag is accepted as an unknown field. The Movement pane adds one theme-spacing unit above the shore-jump option and above dive/cannonball options, keeping each group's key hints close to its checkbox. Inspect the bottom bar and both vertical gaps in default FR and narrow EN demo captures; explicit custom toolbar additions must survive save/login, and Restore defaults must remove both again.

Further gesture spacing (2026-10-19): both gaps above shore-jump and dive/cannonball options now use two theme-spacing units. Keep the associated key hints close to each option and verify small-viewport scrolling.

Optional water controls validation (2026-10-19): full `scripts/check.ps1` green, 3,575 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. The toolbar regression verifies absent defaults, catalogue availability, manual bottom/side placement, persistence with the retired flag, unchanged login and Restore defaults. The legacy-account migration regression confirms no inserted water button. Built the default dev viewer and inspected `.scratch/ao-optional-water-controls-fr.png`, `ao-optional-water-controls-narrow-fr.png` and `ao-optional-water-controls-narrow-en.png` (last at 1024x768), compared with `ao-movement-presentation-fr.png`. The bottom toolbar omits both water modes; the two gesture groups keep visible gaps and readable default/custom keycaps, with normal vertical scrolling in the short viewport. Last-five before/default/rebound FR medians: 60 FPS throughout, UI 0.66/0.64/0.64 ms and GPU 1.83/1.68/1.73 ms; shared desktop and demo state vary, so this is a regression observation rather than an isolated speed comparison. Owned captures have no errors/panics and only the baseline synthetic outfit warning. Human acceptance remains required before PR.

Movement presentation validation (2026-10-19): final `scripts/check.ps1` green, 3,575 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. The pure passage-binding regression covers derived Ctrl chords, modifier filtering, secondary-only explicit mouse bindings and unassigned controls; the existing rendered regression also checks standalone modifier keycaps. Built the default dev viewer and inspected `.scratch/ao-movement-presentation-fr.png`, `ao-movement-presentation-narrow-fr.png` and `ao-movement-presentation-narrow-en.png` (last at 1024x768), compared with `water-crouch-controls-fr.png`. The inner child scope preserves the gutter even though the parent columns have fixed minimum widths. Surface/underwater order, shared keycaps, switch alignment, custom mouse alternatives and small-viewport scrolling are correct. Last-five before/default/rebound FR medians: 60 FPS and UI 0.66 ms throughout; GPU 2.05/1.83/1.84 ms. These are shared-desktop regression observations, not an isolated performance comparison. Owned logs contain no errors/panics and only the baseline synthetic missing-outfit warning. Human acceptance remains required before PR.

Passage / water-walk validation (2026-10-18): full `scripts/check.ps1` green, 3,573 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. Built the default dev viewer and inspected `.scratch/ao-passage-layout-fr.png`, `ao-passage-layout-rebound-fr.png` and `ao-passage-layout-small-en.png`, compared with `ao-passage-layout-before-fr.png`. Both switches align beneath the run/flight boosts, concise passage text fits its column, default/rebound hints remain current, and swimming controls remain accessible at both viewport sizes. Last-five before/after FR medians: 60 FPS, UI 0.72/0.69 ms and GPU 2.04/2.09 ms; shared desktop load varies, so these are regression observations. Owned logs contain zero errors/panics and only the same synthetic missing-outfit warning. Human acceptance remains required before PR.

Other existing AO modes: interface, interface-group, interface-track, interface-combine, interface-swim, interface-swim-missing, folder-names, folder-names-wait, 1, transitions, off, movement, movement-rebound, swim, swim-missing, firestorm, firestorm-wait, migrate, migrate-done, migrate-swim, migrate-alchemy, migrate-alchemy-done, empty, design-current, design-tabs, design-movement, design-section, design-swim.


- **`AURORA_DEMO_ANIM_LOOP=1`**: loop whose first interval is missing,
  sequence changed every 120 frames, stop / restart at frames
  960 / 1020 every 1200 frames.
- **`AURORA_DEMO_ANIM_LOOP=resync`**: the delayed-loading pair below, with
  the default Legacy animation clock. Frame 750 presses the bound Ctrl+S
  action to restart both residents locally. The log reports the elbow phase
  error before (frame 720) and after (780 and 900, expected below 0.001 rad,
  including later sequence changes). Capture at 740 and 800. On the grid, start
  a group dance, press Ctrl+S and check the local restart/alignment; repeat
  from World › Resync animations and the corresponding toolbar button.
  HUD script permissions, seating and other viewers must remain unchanged.
  Ctrl+S in a notecard or script editor must save without restarting dances.
- **`AURORA_DEMO_ANIM_LOOP=sync`**: enables Synchronized clock in the demo
  settings (normal default: Legacy). The two avatars receive distinct animations
  together at frame 120; the second file becomes available only at frame 300.
  Both must then show the same loop phase, including subsequent sequence changes
  and stop/restart. Capture after frame 360; the log reports the elbow phase error
  then (expected below 0.001 rad). Unit regressions additionally skip
  over ten minutes of skeleton evaluation, preserve the loop introduction and
  exercise delayed loading across an explicit attachment-edit freeze. On the grid,
  have two avatars dance, turn the camera away and back, move out of range and
  return, then repeat after ten minutes; also check normal jump completion.
- **`AURORA_DEMO_ANIM_LOOP=sync-edit`**, with `AURORA_DEMO_ANIMESH=1`: the same
  delayed-loading pair, then real build selections freeze our worn rigged mesh
  (frames 420–630) and a ground animesh (750–930). Capture 600/700 and 850/1000:
  the selected skeleton holds its pose, then rejoins the uninterrupted reference.
  The log reports avatar phase error at 660 and animesh phase error at 960
  (expected below 0.001 rad). The edit freezes the displayed pose only; receipt
  origins stay unchanged, including signals and assets received during editing.
  Both synchronization scenarios enable Preferences › Aurora › Animations ›
  Synchronized clock without changing normal-mode settings. On the grid, explicitly
  enable this choice before testing synchronization. Legacy must remain the
  default for fresh and older settings files, start newly loaded animations
  from the beginning and resume the frozen pose after editing. Switching live
  to Synchronized clock must catch up both avatars and animesh; switching back
  must preserve their current phase and suspend again during subsequent edits.
  Check persistence after reopening the viewer, both bulk Aurora/Legacy choices,
  and Reset this section restoring Legacy.
- **`AURORA_DEMO_AO=interface|interface-group|interface-track|interface-combine|interface-swim|interface-swim-missing`**: approved native AO interface. `interface` has six synthetic standing entries; `interface-group` opens a simultaneous group, `interface-track` an independent cycle, `interface-combine` its guided creation dialog, and `interface-swim` one selected swimming slot with all eight resolved. `interface-swim-missing` removes Floating from that fixture: its state row must show an amber warning triangle, assigned required states a green check, and optional states no marker. The detail pane, compact AO panel and Movement gate share the triangle and missing-animation message; tooltips distinguish missing and assigned required states. No optional-slot counter remains. Test FR/EN, state navigation, single-click selection, row drag insertion/reload, creation from checked main entries, group-member add/remove/order, track cycle isolation, rename and dissociation. Inventory operations use the normal mutation/acknowledgement path against the offline simulator. Examples never seed a user's inventory; all shared theme/font/widget files remain unchanged.
- **`AURORA_DEMO_AO=folder-names`**: seeds an existing native AO (including all eight swimming slots) and an empty bridge directory under their previous `AO` / `Bridge` names. The real acknowledged startup edits must rename both to `#Aurora/Aurora AO` and `#Aurora/Aurora Bridge`; the AO selection and links remain unchanged. Opens the real inventory beside the AO after frame 360 with both sibling folders visible; capture at 1050 after bridge provisioning. Regression tests additionally cover an already worn current bridge retaining its folder/item/object identity and valid handshake without another copy/upload/detach, new-name precedence, pending/failed acknowledgements, an authoritative empty old AO and old interrupted-import protection. Human grid validation: confirm the two existing folders are renamed on login, the selected set and animations still work, and a second login creates no duplicate folder or legacy AO import.
- **`AURORA_DEMO_AO=folder-names-wait`**: uses the same cached old native folders but withholds both inventory services until frame 720. Capture at 450: `AO` and `Bridge` are unchanged, the selected set still works and no metadata edit is sent. Capture at 1200: both folders have their new names and the bridge reaches Ready after its five-second inventory save. The factory must wait beyond its normal timeout if services are delayed, rename an already worn controller without replacing its identities, and still time out a request that was actually sent. The readiness gate is shared with AO startup; running/ready bridge commands continue independently of inventory availability. Offline only.
- **`AURORA_DEMO_AO=1`**: opens the AO manager, with two standing poses in
  a cycle, walk, run, sit, fly, typing and an importable notecard.
  The movement keys change the state. `transitions` chains the
  states automatically after loading, every 180 frames;
  `off` starts with the AO disabled. Check the AO button at the bottom, the
  poses, the pause, the cycles and adding / importing from the inventory.
- **`AURORA_DEMO_AO=movement|movement-rebound|swim`**: test the actual integrated AO window. `movement` opens its persistent Movement (EXPERIMENTAL) tab, with a matching experimental heading and menu/toolbar/keybinding labels; `swim` opens the selected generic swimming list with synthetic contributed inventory links and expands only this owned offline window so the lists and cycle controls can be inspected; normal users retain their saved window size and scrolling. `swim-missing` opens Movement with one slot absent: swimming must remain unavailable with the shared warning triangle and missing-animation message. Confirm independent run/flight/swim/passage toggles, run and flight multipliers starting at x1, the active toolbar indicator, swim-state navigation, AO enable/pause, and animation choice/cycles. Ctrl+Alt+M and the movement toolbar open this same manager. Passage uses Ctrl plus the current jump keys (default E / Page Up); it requires its own switch, leaves text/media focus alone, and yields to explicit shortcut conflicts. Rebind Jump and confirm the displayed chord follows. Disable passage during its pulse to cancel it. In a normal human-driven grid session, assign all eight swimming slots in the selected set, enable swimming and verify another viewer sees the matching idle, movement, cushioned-entry and separate flight/shore exit animations. Ordinary AO disable/pause must not stop active swimming animations. Removing a slot or selecting an incomplete set must stop swimming; a different complete set cannot unlock that selection. No import-pack button or automatic fill/upload prompt may appear. Capture around frame 850.
- **`AURORA_DEMO_AO=design-current|design-tabs|design-movement|design-section|design-swim`**: a native interactive layout study. The Current view reuses the existing AO body unchanged; the side workbench switches to Animations/Movement tabs or a collapsible section. Ctrl+Alt+M and the movement toolbar button open the prototype Movement tab and reopen its workbench; the separate movement floater stays hidden in this design scenario. Movement switches/sliders are temporary preview values and do not control motors or write account movement preferences. `design-swim` opens the AO swimming list and its matching avatar preview. The workbench selects simulated avatar animation states; swimming selection uses the actual AO state lists, including the eight separate swimming contexts in the synthetic inventory. Check AO activation/pause, state selection, animation double-click/cycles, independent motor switches and reset, and returning from movement controls to swimming animation selection. All inventory changes stay in the offline demo. Capture each layout around frame 850; keep the existing dimensions, palette, fonts and widgets for comparison.
- **`AURORA_DEMO_ANIMESH=1|churn|alpha|alpha-cycle|points`**: animated root mesh, linked mesh flagged
  by a child and worn animesh, with independent skeletons of their own and
  alpha shadows. `churn`: the animesh on the ground (orange) also changes its
  mesh asset every 240 frames, like the rigged meshes that come and go on
  the grid; it must stay whole and animated at each change, the other
  two as well, and the line `demo animesh: skin bindings <in use> used
  of <in the list>` of the log (every 120 frames) must stop growing
  once the first meshes are freed (600 frames after their last use).
  `alpha` adds zero-alpha **non-rigged mesh** children 9205/9206 to the
  ground/worn animated linksets. These boxes must appear with
  `AURORA_DEMO_DEBUG=alpha` alone, independent of "Rigged objects too".
  It also makes linked mesh 9202 fully transparent, retaining its skin and
  animation. With `AURORA_DEMO_DEBUG=alpha,alpha_rigged`, it must appear in
  red and follow its skeleton; the invisible non-rigged root 9201 must
  appear with `alpha` alone. Toggle transparency off and on after loading:
  both must disappear and reappear. Turning off "Rigged objects too" hides
  only the rigged overlay. Normal rendering and shadows stay invisible.
  `alpha-cycle` uses the same fixture and switches the overlay after normal
  loading: frame 720 on with rigged faces, 960 off, 1200 on without rigged
  faces, 1440 on with rigged faces. Capture at 850,1100,1350,1550. The
  `demo animesh alpha` log gives temporary records and red draws per object:
  mesh 9202 must have red draws only with rigged highlighting, meshes
  9205/9206 (`rigged false`) and root 9201
  whenever the overlay is on, and both must have zero records when off.
  `points` (#1492) adds to the worn animesh 9203 a child prim 9207, a red
  bar rigged to the attachment point `Groin` alone, and keys that point in
  the demo animation (a 0.9 rad swing about Y, 2 s loop): attachment points
  are joints the skin and the animations reach, as in the Legacy Viewer,
  so the bar swings with the point. `AURORA_DEMO_TOOLBOX=0:J9` with
  `AURORA_DEMO_POS=134.5,128` and `AURORA_DEMO_CROWD=4` lists `Groin` among
  the bar mesh's joints (not as unknown) and in the control skeleton with a
  turning rotation. A green stem (prim 9208) is rigged between `Avatar
  Center` (its back half) and `mPelvis` (its front half), and the animation
  moves the pelvis by position (0.3 m forward, 0.2 m down, 2 s loop): the
  back end stays on the root while the front end follows the pelvis.
- **`AURORA_DEMO_ANIMESH_TRIS=<n>`**: with `AURORA_DEMO_ANIMESH`, adds
  about n triangles to the demo's rigged mesh, as opaque faces of 45,000
  (each a thin tube around the torso, 7 faces at most). With
  `AURORA_DEMO_CROWD=<n>`, each avatar of the crowd wears this mesh: 200000
  gives avatars as heavy as those of the grid (3.6 million
  triangles on screen for 16 avatars drawn in full), which is what is needed
  to measure the passes that redraw the avatars (shadows, prepass).

Rigged mesh compatibility (#586): the `rigged_skin_parity` regression tests
exercise unknown-joint pelvis fallback, preservation of inverse binds and
bounded vertex indices. Check ordinary worn meshes and ground/worn animesh
with `AURORA_DEMO_ANIMESH=1`. Human grid validation must compare the reported
Solarian Feline with the Legacy Viewer: the face at rest and in motion,
near/far LOD changes, detach/reattach and the appearance previews. Synthetic
tests do not establish that this report uses an invalid joint or index.
- **`AURORA_DEMO_CLOUD=1|self`**: `1` enables offline the clouds and the
  loading bars. `self` also dresses our avatar like those that
  stayed a cloud for the 60 s of the limit (issue #121): invisible head
  bake (alpha layer), textured upper and lower body, a worn mesh that the
  asset server does not have (404) and a hat. It must appear about
  1 s after its arrival (its worn textures are all there; 3 s at most
  otherwise), without a head and without the mesh, and the log must
  contain "avatar … (own) shown after 1.0 s", then the time each part
  completed. Screenshots with `AURORA_FPS_LIMIT=60` and
  `AURORA_CAPTURE_FRAMES=240,1200`: the avatar is already shown at 240.
- **`AURORA_DEMO_CLASSIC=1`**: the two avatars of the plaza become classic
  avatars, the system body with a skin bake whose alpha is soft in a band
  at the waist and cut by a row of holes (an alpha layer), and a hair bake
  in strands that fade out. Each has a shape far from the default one: ours
  male with long hair at the back, the wolf heavy (body fat, belly) with
  big hair and pigtails. The bodies must be solid (the band at
  the waist shows nothing of what is behind), cut out by the holes, each
  with its own build; the hair shows its strands and what is behind them
  where they fade. With `AURORA_DEMO_CAMERA=close,640,500,45` and
  `AURORA_CAPTURE_FRAMES=700` (1280 × 800): our avatar's back and hair in
  close-up; with `AURORA_DEMO_POS="133.6,127.9"`, `AURORA_DEMO_CAMERA=ml`
  and frame 660: the wolf from the front (a grey silhouette, it is over the
  complexity limit of the demo, but with its build).
- **`AURORA_DEMO_PHYSICS=1`**: our avatar and the one in front of it wear a
  physics layer and a rigged mesh of four pink balls weighted to the
  LEFT_PEC, RIGHT_PEC, BELLY and BUTT collision volumes, as mesh bodies are.
  From frame 240 both turn their side to the camera and hop twice every two
  seconds, their chest rolling from side to side: the balls overshoot at each
  landing, sway, then settle before the next hop (capture after frame 1300,
  when the rez effect is over). The log writes the volumes' offsets every 6
  frames from frame 230 to 1200 (`demo physics: frame …`). `still`: the
  same turn without the hops, the offsets stay constant (the layer's gravity
  sag). `off`: the same hops with Preferences › Graphics › Avatar physics
  off, no offsets logged and the balls follow the body rigidly. `classic`:
  the same hops without the balls, the system body's own breasts, belly and
  butt bounce (its vertex morphs of the driven parameters). Compare the
  frame-rate independence with `AURORA_FPS_LIMIT=30`, `60` and `144`: the
  offsets at the same frame time match.
- **`AURORA_DEMO_DEFORM`** (issue #272): at frame 240, a one-second pose
  that moves the neck up and the shoulders out plays once on the avatar in
  front of the start position ("Orion"), then stops. As on the grid, the
  joints keep the positions it gave them: the neck stays stretched, the
  shoulders wide and the plate higher. Right click on the avatar, "Reset
  skeleton": it gets its shape back at once. `reset` / `reset-anim` choose
  "Reset skeleton" / "Reset skeleton and animations" by themselves at
  frame 700; `self` deforms our own avatar instead, `self-reset` resets it
  at frame 700 (the log shows `skeleton reset: <id>`). Screenshots with
  `AURORA_FPS_LIMIT=60` and `AURORA_CAPTURE_FRAMES=650` (deformed) or
  `780` (reset).
- **`AURORA_DEMO_EMOTE`** (issue #661), with `AURORA_DEMO_CLASSIC=1`: from
  frame 240 the avatar in front of the start position plays a looping
  animation whose header names an expression and a hand pose, as uploaded
  animations do (it holds mFaceRoot at rest: as in the Legacy Viewer, an
  animation without joints would only hold the hands while it eases in).
  Every 240 frames it is off for one frame
  and starts again, which plays its expression again: it eases in for
  0.3 s, holds, then eases out by 2.8 s, while the hands keep the pose. The
  expression is the animation state without `express_` (`smile`,
  `toothsmile`, `wink`, `open_mouth`, `kiss`…; `_emote` may be left out,
  `toothsmile` by default), the hand pose its index in the upload window's
  list (0 spread, 1 relaxed, 3 fist by default … 13 spread right).
  `signal` signals the expression's built-in UUID beside the animation, as a
  gesture does; `self` plays it on our own avatar, which
  `AURORA_DEMO_SHAPE=mouth` (face) or `torso` (hands) frames from the front:
  with `AURORA_FPS_LIMIT=60` and `AURORA_CAPTURE_FRAMES=760` the face smiles
  (eyes narrowed, teeth showing) and the fists are closed; without the
  variable the same frame shows the face at rest and the hands relaxed (the
  demo idle animation asks for them, as LLHandMotion does when no animation
  asks for a pose). The log shows `demo emote: round <n> on <avatar>`.
  Breathing turns the chest and the body noise sways the torso by about a
  degree whenever no stronger animation drives them.
- **`AURORA_DEMO_DERENDER`**: the avatar in front of the start position
  (Violet Wolf, unblocked for the scenario) wears a hat and an orb. At
  frame 700 its menu opens (`menu`: Inspect, Derender temporarily and
  Derender & blacklist enabled, Script information greyed: it needs the
  LSL bridge). `inspect` chooses "Inspect" at frame 740: the Inspect
  window lists the hat and the orb. `avatar` chooses "Derender
  temporarily" at frame 740: the avatar, its hat, orb and name plate
  vanish; the Asset blacklist window opens at frame 800 with the resident,
  and "Clear temporary" at frame 900 brings everything back. `hat` opens
  the menu of the hat and chooses "Derender & blacklist": only the hat
  vanishes; the window lists it as a permanent object (written to the demo
  account's `asset_blacklist.xml`), removed at frame 900, which brings it
  back. By hand: blacklist something, quit and relaunch the demo: it stays
  away until removed in World › Asset blacklist. Screenshots with
  `AURORA_FPS_LIMIT=60`, `AURORA_DEMO_CAMERA=close,830,330,40` (on the
  avatar) and `AURORA_CAPTURE_FRAMES=720,780,830,950`.
- **`AURORA_DEMO_SEAT_KILL=1`** (issue #746): Violet Wolf, with its hat and
  orb, sits on a violet cushion at frame 700. The cushion is deleted at
  frame 800, before the simulator stands Wolf up at 900, as a carry or pose
  object that dies does. Wolf must stay where it sat, hat and orb on, then
  stand on the plaza; only the cushion goes. Screenshots with
  `AURORA_FPS_LIMIT=60`, `AURORA_DEMO_CAMERA=close,830,330,40` and
  `AURORA_CAPTURE_FRAMES=760,850,960`.

Concurrent object editing: `build-edit` opens the real Build menu while IK
is active, `build-before` reopens IK while preserving a selected linked prim,
`build-pick` directly designates Right while Left remains edited, and
`build-moved` moves the edited target upward by 8 cm. Both floaters stay open;
the skeleton remains behind them. A scoped diagnostic at frame 600 reports
both open states and the selection count. Capture at frame 620.

## Rendering and environment

| Variable | Effect |
|---|---|
| `AURORA_DEMO_PARCEL_VISIBILITY=<mode>` | Parcel content isolation and same-owner neighbours |
| `AURORA_DEMO_PARTICLES=<mode>` | Wind and smoke particles, region wind comparison, a leash chain, or particle load |
| `AURORA_DEMO_PLANAR=1` | Slabs with planar texture mapping |
| `AURORA_DEMO_PBR_OVERRIDE=1` (or `alpha`) | GLTF material override (`alpha`: overrides with a zero base color alpha) |
| `AURORA_DEMO_PBR_METAL=1` | White PBR metal, polished to rough, and a cube with an occlusion map |
| `AURORA_DEMO_PBR_EMISSIVE=1\|sided` | Emissive PBR spheres or transparent single-/double-sided faces |
| `AURORA_DEMO_BOUNCE=1` | A white floor and box between a red and a green wall (GTAO's indirect light) |
| `AURORA_DEMO_SHIMMER=1` | A long glossy plate with a fine repeated normal map (specular anti-aliasing) |
| `AURORA_DEMO_TEXANIM=1` | Texture animations (llSetTextureAnim) |
| `AURORA_DEMO_GLOW_LAYERS=1` or `shell` | Transparent glowing layer over a glowing skin |
| `AURORA_DEMO_PROJECTOR=1` or `point` | Projectors (lights with a projected texture) |
| `AURORA_DEMO_SKY=<gamma>[,<haze>[,<ambient>[,<probe ambiance>]]]\|midday` | EEP sky with this gamma, classic unless it has a probe ambiance; or the library Midday |
| `AURORA_DEMO_WATER_EEP=<fresnel offset>,<fresnel scale>,<blur>,<normal scale>[,<fog r,g,b>]` | Water of the `AURORA_DEMO_SKY` day with these values, out-of-range ones included |
| `AURORA_DEMO_SKY_OPTICS=rainbow\|halo\|clear` | Rainbow or 22° halo in the sky |
| `AURORA_DEMO_NIGHT_SKY=real\|legacy[:<azimuth>,<elevation>]` | The night sky's stars at midnight, the camera looking at the sky |
| `AURORA_DEMO_NIGHT_SKY_PLACE=<latitude>[,<gain>]` | Where the real sky is seen from, and its stars' brightness |
| `AURORA_DEMO_OCCLUSION=1` | Wall with hidden objects (occlusion test) |
| `AURORA_DEMO_DEBUG="<overlays>"` | Debug overlays |
| `AURORA_DEMO_EEP_PARCEL=<mode>` | EEP environment of the region and the parcel |
| `AURORA_DEMO_ENV_SELECT=<mode>` | Environment picker |
| `AURORA_DEMO_ENV_EDITOR=<mode>` | Sky, water and day cycle editors |
| `AURORA_DEMO_ENV_KEYS=u\|y\|n\|x\|j` | World › Environment shortcut pressed (Ctrl+Shift + the letter) |
| `AURORA_DEMO_DOF=<f>:<m>[:debug]` | Depth of field at this aperture and focus distance |
| `AURORA_DEMO_AA_SWITCH=<frame>:<mode>[,…]` | Anti-aliasing changed at these frames, as Preferences would |

### Details

- **`AURORA_DEMO_AA_SWITCH=<frame>:<mode>[,<frame>:<mode>…]`**: the
  anti-aliasing changes at each frame given (modes of `AURORA_AA`). A change
  of MSAA samples, or of SSR, compiles the scene's pipelines on a worker
  thread (#1746, #1722): the log gives `compiling the pipelines for <n>
  sample(s)` then `pipelines for <n> sample(s) ready in … ms`, and with
  `AURORA_PROFILE=1` the `perf summary` of that second shows no long frame
  (on an RTX 5070: `max=16.68`, against a 146 to 155 ms frame when they were
  compiled on the render thread). The frames keep the previous samples until
  the set is ready; `AURORA_CAPTURE` waits for it. For example
  `AURORA_AA=3`, `AURORA_DEMO_AA_SWITCH=700:4,1000:2,1300:3` and captures at
  650, 900, 1200 and 1500 (MSAA 4x, 8x, 2x, 4x).

- **`AURORA_DEMO_PARCEL_VISIBILITY=off|always|listed|except|except-listed|same-owner|same-group|group-owned-current|always-visible|lists|crossings`**:
  four parcels split the plaza at x/y 128 m. The northern parcels share an owner;
  the southern ones belong to someone else. `always`, `listed` (current parcel
  listed) and `except` (empty list) hide content outside the north-east parcel;
  `off` and `except-listed` show everything. `same-owner` also shows the north-west
  neighbour. Terrain and our avatar/HUDs remain visible. Combine with
  `AURORA_DEMO_POS=124,132` to stand on the north-west parcel and look towards
  the plaza on its same-owner north-east neighbour: hidden with `always`,
  visible with `same-owner`.
  `always-visible` lists the north-east parcel as an explicit rendering exception,
  with both automatic neighbour options off. `same-group` assigns a group to the
  north-west parcel and deeds the north-east neighbour to that group, with the
  same-owner option off; `group-owned-current` reverses assigned/deeded status.
  The northern plaza stays visible in both group scenarios from `124,132`.
  `lists` fills both saved lists with three long synthetic parcel names. Combine
  with `AURORA_DEMO_OPTIONS=graphics:search=parc` to inspect the full-width lists,
  wrapped names, separate region lines and aligned remove buttons, including in
  a narrow window or at an enlarged UI scale. Larger lists scroll independently.
  `crossings` starts on the north-west parcel with filtering enabled only there
  and the north-east plaza always visible. From frame 700 it alternates between
  home and the south-west parcel every 120 frames, delaying arrival parcel data
  by 12 frames. Every frame from 650 checks that the plaza remains visible and
  that the filter changes only with the completed arrival decision. Phase logs
  record visibility, filtering and the scene version. Capture at frames 680, 760
  and 1160 to compare home, outside and repeated return; no crossing errors should
  be logged. Owner/group discovery also publishes its complete result at once.
  Combine with
  `AURORA_DEMO_OPTIONS=graphics/6` for the native controls in Graphics > Parcels,
  after Smoothness. Check live mode changes,
  adding/removing scope entries and always-visible parcels, and restoring content
  without relogging. To add a neighbouring exception without walking there, select
  it in About Land, then use "Add selected parcel" in the always-visible list.
  On the grid, also check parcel changes, listed parcels in different regions,
  assigned/deeded groups on either side, foreign-owner exceptions, connected region
  borders and moving linked objects. Owner and group options combine by union;
  missing groups do not match, and exceptions do not extend automatic discovery.
  Parcel isolation is local rendering; it does not stop simulator object updates.

- **`AURORA_DEMO_DOF=<f-number>:<distance>[:debug]`**: the depth of field on
  whatever Preferences say, at that aperture, focused at that distance (m,
  along the view axis, no transition). At frame 600 the log gives the
  scene's distances: `dof demo: … own avatar at 3.3 m, other avatars at
  7.9 m` (the foreground, the subject with its panels; the sphere and the
  hills are the background, 15 to 40 m). Compare `1.4:3.3` (the avatar
  sharp, the rest blurred), `1.4:7.9` (the subject sharp, the avatar a soft
  blur that spreads over it) and `8:7.9` (barely any blur), under MSAA
  (`AURORA_AA=3`) and SMAA (`AURORA_AA=6`): no halo around a sharp edge in
  front of a blurred background, no staircase on a blurred foreground's
  edge. `:debug` paints the circle of confusion: near red, far blue, sharp
  grey.
- **`AURORA_DEMO_PARTICLES=wind|smoke|both|legacy`**: wind streaks of the
  motorbike and parameters of the smoke script received through compressed
  updates, in front of a dark panel; extended glow format (`legacy`:
  wind without glow), default soft texture of the particles offline.
  `region-wind`: two otherwise identical six-second smoke emitters in front
  of the panel; "Wind" has `LL_PART_WIND_MASK`, "Smoke" does not. The
  region's wind blows towards negative Y at 6 m/s after the Legacy wind
  scale: only the first plume drifts sideways. `region-calm` uses the same
  emitters with zero wind: both keep their original motion. Override either
  wind with `AURORA_DEMO_WIND=x,y` (raw grid values, scaled by two), for
  example `0,3` to reverse the drift. No grid connection is needed.
  These two modes frame the plumes automatically and close the demo's
  Inventory/AO windows so saved camera preferences cannot hide the comparison.
  `leash`: a leash script in front of the panel, a white ribbon chain from a
  small cube to a teal "hand" cube 5 m away, sagging under gravity, two links
  per burst and a 2.5 m y scale (ribbons ignore it). It must look like a
  thin chain; tall vertical strips are the links emitted together drawn as
  billboards (#1049).
  `load`: instead, 32 more fountains in a circle around the plaza,
  that is about 4,000 live particles, to measure the `p_part_*` parts
  of the profile (combine with `AURORA_DEMO_STREAM=<n>` for the
  number of objects).
- **`AURORA_DEMO_PLANAR=1`**: the tiles must line up from one slab
  to the next.
- **`AURORA_DEMO_PBR_EMISSIVE=1`**: three dark spheres ahead of the start
  position with the same orange emissive (PBR), from the right: opaque,
  alpha-masked and blended. All three glow orange; with `AURORA_HDR=0`
  ("HDR and Emissive" off) the opaque and masked ones turn dark grey and
  only the blended one keeps its light, as in the Legacy Viewer.
  With `sided`, four isolated translucent cyan faces stand ahead of the
  start position, in increasing world Y order: single-sided front/back,
  then double-sided front/back (#1727). Seen from the start (lower X), the
  single-sided back must be absent and both double-sided faces must be
  visible. Move to the other side to reverse the front/back views: again
  exactly three faces must show. These are isolated faces, with no opposite
  wall that could conceal missing back faces.
- **`AURORA_DEMO_PBR_METAL=1`**: three white metal spheres (PBR, metallic 1,
  roughness 0.05, 0.3 and 0.6) and a polished slab ahead of the start
  position. A white metal has no colour of its own: the polished one
  mirrors the place and the sky, the rough ones their blur. Compare
  with the sky behind them: the metal is as blue as the sky it reflects,
  no more (`AURORA_DEMO_SKY=1,37` for a paler sky). Left of the spheres, a
  polished metal cube whose packed map (occlusion, roughness, metal) has
  an occlusion of 0 but for a disc on each face: the cube is dark and only
  reflects the sky in the discs, as in the Legacy Viewer
  (`AURORA_PBR_OCCLUSION=alt` shows it plain one second in three). Left
  of the cube, an inside corner (floor, back and side) of the
  half-polished metal: with GTAO its reflections fade into the corner
  (`AURORA_GTAO_SPECULAR=alt`; seen best with `AURORA_DEMO_POS="131.5,131"`).
- **`AURORA_DEMO_SHIMMER=1`**: ahead of the start position, a 16 × 6 m
  glossy plate (legacy material, specular exponent 245) whose bump normal
  map is repeated 48 × 18 times, seen at a grazing angle. With
  `AURORA_SPECULAR_AA=alt` and a moving camera
  (`AURORA_DEMO_CAMERA=drift,620,600,1`), compare its far highlights and
  the rims of the polished spheres (with `AURORA_DEMO_PBR_METAL=1`) from one
  second to the next.
- **`AURORA_DEMO_BOUNCE=1`**: left of the start position, a white floor
  between a red wall and a green one, with a white box against them
  (seen with `AURORA_DEMO_POS="131.5,134.5"`). With GTAO's indirect light
  (`AURORA_SSIL=alt`) the box's sides and the foot of the walls take the
  light of the floor and of the walls near them.
- **`AURORA_DEMO_PBR_OVERRIDE=1`**: two PBR slabs, one of them with a
  GLTF material override (4 × 4 repeats, tint). Their normal map,
  also carried by the "Normal + specular" panel of the ordinary
  demo, contains only bumps: each must be light on the side
  of the sun (at the top of the panel with `AURORA_DEMO_TOD=2`). Dark at the
  top and light at the bottom, the relief is inverted.
  With `alpha`, three upright panels of the same opaque material, in
  front of `AURORA_DEMO_POS="131,128"`, get an override with a zero base
  color alpha: the opaque one stays drawn (an opaque glTF material ignores
  its alpha), the masked one is cut out by its cutoff and the blended one
  is not drawn, only their labels show. The opaque one missing is the bug of a worn mesh attachment that
  the Legacy Viewer draws.
- **`AURORA_DEMO_TEXANIM=1`**: two rows of panels in place of the
  alpha panels — smooth scrolling, grid of 4 × 4 frames, ping-pong,
  rotation, scale (alpha mask: prepass and shadows), a cube animated on
  a single face, a legacy material (the normal map follows) and a PBR face.
- **`AURORA_DEMO_PROJECTOR=1|point`**: four lights, three of them
  projectors: a white projector facing a wall at 4.5 m, which projects
  the demo image (four colour quarters around a white disc), with
  a post in its beam; a cyan projector 2.5 m above the ground facing
  down, with the plain white image (blur from 1 m, ambiance 0.3);
  a pool light, a very wide cyan projector (field of view 3.0 rad, radius
  20 m, no falloff) facing down, 2 m below the ground and 4 m from the
  middle of a slab with a normal map; an ordinary point light,
  orange, at the end of the wall. To look at with `AURORA_DEMO_POS="131,112"`,
  in `AURORA_AA=6` and `AURORA_AA=3`: the image is projected on the wall
  (green top left, yellow top right, red and blue below: the top
  of the image follows the prim's Y axis), the ground is lit only inside the
  cones, and the pool light lights nothing above it: neither the slab nor
  our avatar. The square of light of the projector with the white image has
  soft edges (visible with `AURORA_DEMO_TOD=3`): the fade comes from the
  distance to the focal point, not from the levels of the texture. The post
  casts its shadow on the wall: the two projectors most present on screen
  have a shadow map (`AURORA_PROJECTOR_SHADOWS=0`: none, the wall is
  lit behind the post), and their shadows fade in
  during the first second. `point`: the three projectors without their
  texture, hence point lights, as they were rendered before being supported —
  the pool light then lights in cyan our avatar from below and
  the sides of the slab's relief turned towards it. The orange light is
  the same in both modes.
- **`AURORA_DEMO_GLOW_LAYERS=1|shell`**: two "skin" panels (the first
  with a glow of 0.02, the second without) and, 1 cm in front of their lower
  half, a transparent pink layer (alpha from 0 to 1) with a glow of 0.02, like
  the tattoo layers of mesh bodies. To look at with
  `AURORA_DEMO_POS="131,112"` and `AURORA_DEBUG_GLOW=1`: under the layer, the
  glow follows its alpha (from 0 to 0.02) on the skin without glow and stays
  at 0.02 on the other, without adding up. `shell`: the layer also shows its
  other faces; the back one, seen from behind, must add nothing (same values
  as with `1`).
- **`AURORA_DEMO_WATER_EEP=<fresnel offset>,<fresnel scale>,<blur>,<normal scale>[,<fog r>,<fog g>,<fog b>]`**:
  the water of the EEP day `AURORA_DEMO_SKY` gives the region, with these
  values as a region's asset carries them (the normal scale on its three
  axes); a missing or unreadable value keeps the default (0.5, 0.3999,
  0.04, 2, fog 0.0156 / 0.149 / 0.2509). Values beyond the Legacy Viewer's
  ranges are clamped to them like LLSettingsWater's validation (unit test
  `water_values_are_clamped_to_the_legacy_ranges`; the image does not show
  it: a fresnel offset clamped to 1 still mirrors the whole sky, so
  `AURORA_DEMO_WATER_EEP=1.8,0,0,2,1.6,1.6,1.6` under a white midday sky is
  a white sheet before and after). The reported region's water: with
  `AURORA_DEMO_TOD=0 AURORA_DEMO_SKY=1.6,37,1.2
  AURORA_DEMO_WATER_EEP=0.5,0,0,0,0.101,0.211,0.196` (a hazy midday sky; a
  normal scale of 0, a fresnel scale of 0 and no blur) and
  `AURORA_DEMO_WATER=off:shore`, the sea is dark and hazed toward the
  horizon, without any reflection, as in the Legacy Viewer (its reflection
  normal is normalize(0) there), never a white sheet; with `medium:shore`
  (enhanced water) it keeps its waves and foam and has no reflection
  either: turquoise near the shore, blue toward the horizon, not white.
  Without these variables (the demo's default sky and water), every sea
  takes the Legacy haze and reflection: the distant water is darker than
  before and meets the sky on a sharper line. Compare the Legacy Viewer
  with the same values in Personal Lighting.
- **`AURORA_DEMO_SKY=<gamma>[,<haze>[,<ambient>]]`**: classic EEP sky (without
  reflection probe ambiance) with this sky gamma and a sun colour
  above 1: legacy gamma and object light normalised like
  Firestorm. `haze`: haze distance multiplier (0.8 by
  default; `37` gives a hazy sky like many regions have,
  where what is a few dozen metres away is veiled). With
  `AURORA_DEMO_CROWD=40` and `AURORA_MAX_AVATARS=4`, the impostors must
  be veiled like the avatars drawn in full next to them
  (`AURORA_MAX_AVATARS=60` to compare). `ambient`: ambient light of the
  sky (0.25 by default, more than 1 in the brightest skies), a grey
  level or a colour as `r:g:b` (`0.9:0.6:0.5`, a warm sky); with
  `AURORA_DEMO_SKY=1.62,0.8,1.1` and `AURORA_DEMO_PBR_OVERRIDE=1`, the surround
  of each bump of the slabs must be darkened by ambient occlusion, and no
  longer be once ambient occlusion is turned off in the settings.
  `probe ambiance`: the sky's reflection probe ambiance; above 0 the sky is
  no longer classic and is tone mapped. With `AURORA_DEMO_SKY=1,0.8,0.25,0.6`,
  `AURORA_DEMO_TOD=0` (the region's sky, not a saved preset) and
  `AURORA_DEMO_OPTIONS=0:tonemap` (Graphics, scrolled to the row), the Tone
  mapping choice is enabled;
  World › Environment › Midday (a classic sky in the demo) greys it, with its
  tooltip, and "Use shared environment" enables it again.
  `midday`: the values of the library Midday instead (probe ambiance 1, no
  ambient, haze 0.25 / 0.78 with a distance multiplier of 6.54, half the sky
  in clouds, a bluish sunlight of 1.74 / 1.93), lit as the Legacy Viewer
  lights a sky with a probe ambiance: the white avatar and the white boxes in
  the sun neutral grey (warm beige before), their shaded faces slightly
  blue, the sky above the hill pale; the scene is dim, so the automatic
  exposure stays near its highest, 2.
- **`AURORA_DEMO_SKY_OPTICS=rainbow|halo|clear`**: the region's sky carries a
  moisture level of 1 (`rainbow`, sun 12° up in the west) or an ice level of
  1 (`halo`, sun 14° up in the east), and the camera looks east from behind
  the avatar (`demo_sky.rs`). `rainbow`: a bow over the eastern horizon, its
  top about 30° up, red outside and violet inside, the sky brighter inside
  it. `halo`: a ring 22° around the sun, red on its inner edge, darker sky
  inside it. `clear`: the sky and the view of `rainbow` without the bow, to
  compare. Offline, the textures are stand-ins drawn by the demo (the grid
  sends Second Life's own, `rainbow_id` and `halo_id`); the shape, the place
  and the colors' order are those of skyF.glsl. Capture in `AURORA_AA=3` and
  `AURORA_AA=6`.
- **`AURORA_DEMO_NIGHT_SKY=real|legacy[:<azimuth>,<elevation>]`**: sets
  Preferences › Graphics › Environment › Night sky for this launch (in
  memory), the demo's midnight (unless `AURORA_DEMO_TOD` is also given) and
  the camera above the avatar looking towards `azimuth` (degrees from north
  towards east) and `elevation` (degrees up), north 35° up by default
  (`demo_night_sky.rs`). `real`: the stars over Paris with their colours and
  the Milky Way; `real:0,48.86` puts the Pole Star exactly at the centre of
  the image, with Cassiopeia and the Milky Way above it on autumn nights.
  `legacy`: the Legacy Viewer's random white and pale green stars, no Milky
  Way. With `AURORA_DEMO_TOD=2` (noon) the real sky's stars must vanish in
  the bright sky; with `AURORA_DEMO_TOD=3` (sunset) only the brighter ones
  show. Capture with `AURORA_DEMO_SNAPSHOT=save:1600x900:png:<folder>` for an
  image without the interface.
- **`AURORA_DEMO_NIGHT_SKY_PLACE=<latitude>[,<gain>]`**: the real sky seen
  from `latitude` (degrees north, negative in the south; Preferences ›
  Graphics › Environment › "Sky seen from" set to Custom) and, if given,
  "Star brightness" (0.5 to 4, 2 by default), for this launch. With
  `AURORA_DEMO_NIGHT_SKY=real:180,45` and `-33.87` (Sydney), the Southern
  Cross stands in the south and the Pole Star is under the horizon; with
  `real` and `48.86,1`, `48.86,2`, `48.86,4` the same sky at three
  brightnesses.
- **`AURORA_DEMO_DEBUG`**: comma-separated list among `bounds`,
  `culling`, `lights`, `probes`, `skeletons`, `alpha`, `alpha_rigged`, `wire`, `complexity`,
  `glow`, `glow_view`, `freeze`, `lod`, `shadows`, `ao`, `check_lights`,
  `check_probes`, `pass`: the toggles of the Dev Toolbox (formerly
  Preferences › Debug); `pass` is R3, every face tinted by its pass (green
  opaque, blue masked, red blended, yellow hidden). `shadows` is the view
  of `AURORA_DEBUG_SHADOWS`; `ao` shows the ambient occlusion of the screen
  alone, light grey where nothing is occluded (everywhere when the effect is
  off); `check_lights` and `check_probes` are the checks of
  `AURORA_LIGHT_TILES=check` and `AURORA_PROBE_WALK=check`, asked while the
  viewer runs. One view at a time replaces the image (glow, shadows or
  ambient occlusion). `lod` (Preferences › Debug › Detail
  levels) labels the objects around the pointer with the level they are
  drawn at (0 to 3), the one their distance asks for when it is another,
  and the distance and radius it comes from, the numbers of Firestorm's
  Develop › Render Metadata › LOD Info: green at the finest level, orange
  below it, red while the geometry of the level asked for is not shown.
- **`AURORA_DEMO_EEP_PARCEL=1|parcel|default|stale`**: EEP replies replayed
  as the grid sends them (`demo_eep.rs`) — a region day at dusk then,
  0.4 s later, the agent's parcel with no day (`1`: dusk
  stays), with its own day at noon (`parcel`: 5 s fade), a region
  with no day (`default`: Firestorm's default day asset, the built-in
  default sky offline) or replies for another parcel and
  another region (`stale`: ignored).
- **`AURORA_DEMO_ENV_SELECT=1|lighting|list`**: environment picker
  (`demo_env.rs`), with a library containing an
  "Environments" folder and a Settings folder in the inventory, whose
  settings assets are known locally. `1`: the window opens, then
  a sky is chosen (frame 2600), a water (2900), the next sky with ›
  (3200), a day cycle (3500) and "Shared environment" (3800, 5 s
  fade). `lighting`: "Personal lighting" also opens, a sky is
  chosen (2600) then modified (2900). `list`: the list of skies unfolded
  (2500).
- **`AURORA_DEMO_ENV_KEYS=u|y|n|x|j`**: at frame 300, Ctrl+Shift + the
  letter goes through the shortcut table as World › Environment's
  shortcuts (#1264): `u` Sunrise, `y` Midday, `n` Sunset, `x` Midnight,
  `j` Use Shared Environment. The log line `demo env keys:` gives the action
  found and the time of day after it; capture after the sky's transition
  (`AURORA_CAPTURE_FRAMES=620`). `j` with `AURORA_DEMO_TOD=2` must bring
  back the region's sky (time of day 0).
- **`AURORA_DEMO_ENV_EDITOR=sky|clouds|clouds-picker|sun|beacons|water|item|item-save|day|day-play|day-edit|day-notrans|parcel|region|custom|custom-follow`**: sky,
  water and day cycle editors (`app/env_editor.rs`, `app/env_day_editor.rs`).
  `day`: the day cycle editor opens on the user's "my day cycle" (frame
  400: midnight, the purple sky at 25 %, noon, sunset), the ground track and
  its first key frame selected; `day-play` plays it (a day in a minute,
  from frame 900); `day-edit` brightens the noon frame (900), tries a frame
  at 75 % (950, refused: a frame is there), clones the ground track into
  Sky 3 and selects it (1000), then saves over the item (1100, logged as
  `day cycle saved`). The sky and water editors use the library and Settings
  folder of `AURORA_DEMO_ENV_SELECT`. At frame 400 the editor opens on a
  snapshot of the shown sky (`sky`, `clouds` and `sun` pick the tab) or
  water (`water`), or on the user's "my purple sky" item once its folder
  has loaded (`item`: the sky turns purple at once, Save is enabled).
  `clouds-picker` opens the cloud texture picker too: choose successive textures
  or presets without OK to see the sky update; Cancel restores its opening map.
  `item-save` also raises its brightness and saves over the item (frame
  900), then saves it as "my purple sky, brighter" (1100): both saves
  are logged (`environment settings saved`) and the second item shows in
  the Settings folder.
  The parcel and region scenarios (`app/env_apply.rs`) make the avatar the
  owner of its parcel, give the region a dusk EEP day (as
  `AURORA_DEMO_EEP_PARCEL=1`) and the demo simulator answers ExtEnvironment
  updates: an inline day becomes the parcel's (or the region's) and the
  sky follows. `parcel`: the sky editor on "my purple sky" (400), applied
  to the parcel unchanged, as its asset (900, logged `environment: applying
  to Parcel(1)` without a new day), then brightened and applied inline in
  the region's day (1000, `parcel 1 takes the day`); the footer says
  "Applied to the parcel." and the sky turns purple. `region`: the avatar
  manages the estate; the day cycle editor opens on the region's day in the
  region context (400: skies named "Sky 1000m"…, the timeline in hours,
  "Apply To Region" as the main button), its first frame is brightened
  (900) and committed (1000): the editor closes and the region's sky
  brightens. `custom`: About Land opens on the environment as the parcel's
  owner, whose parcel has "my day cycle" applied, and "Custom" is chosen
  (400): the day cycle editor in the parcel context, "Apply To Parcel" as
  the main button, left to the tester. Add `-refused` (`parcel-refused`…)
  and the simulator refuses inline days: Firestorm's WLRegionApplyFail
  alert with the demo's reason. `custom-follow`: as `custom`, then the
  library's "Dynamic Day" is applied to the parcel (900): the open editor
  follows it (two ground frames instead of four, logged `the editor follows
  it`). `day-notrans`: as `day`, then the library's no-transfer "Annan
  Adored Pink Sky" is loaded at 60 % (900): Firestorm's question about
  making the day no-transfer comes first, with its "Do not show again" box (Preferences › General › Environment turns it back on). `beacons`: the sky editor on its
  Sun & Moon tab with both beacons on: a line from the avatar towards the
  sun (amber) and the moon (violet), named at the bottom left; they go off again when the editor
  closes (a capture's exit skips that: the demo profile keeps them on).

## Load and smoothness

| Variable | Effect |
|---|---|
| `AURORA_DEMO_TEXTURES=<n>` | Texture load test: n cubes |
| `AURORA_DEMO_TEXTURES_CHURN=1` | With `AURORA_DEMO_TEXTURES`: textures renewed |
| `AURORA_DEMO_CROWD=<n>[,busy]` or `<n>,<worn>[,<triangles>]` | Sync load test: n avatars; with `busy` what the sync still places itself, with `<worn>` avatars dressed in rigged meshes |
| `AURORA_DEMO_MATERIAL_ARRIVALS=<seconds>` | The crowd's worn prims and meshes use a legacy material, and a material nobody uses arrives every `seconds` |
| `AURORA_DEMO_IMPOSTOR_FLIP=<seconds>` | The avatar limit alternates between its setting and one every `seconds` |
| `AURORA_DEMO_OVERDRAW=<n>` | Overdraw test: n rows of opaque walls |
| `AURORA_DEMO_STREAM=<n>[,<wave>][,leave][,j2k=<folder>][,net=<ms>][,size=<m>]` | Streaming stutter test; with `net=`, textures downloaded from a local server |
| `AURORA_DEMO_REZ_EFFECT=1\|loop\|styles\|cold` | The rez effect: objects and an avatar appearing, dressing and leaving in front of the camera; `styles`: its three looks side by side; `cold`: a cold arrival with slow textures |
| `AURORA_REZ_EFFECT=0\|1` | The rez effect off or on, whatever the setting (Preferences › Aurora) |
| `AURORA_REZ_STYLE=1\|2\|3` | The rez effect's look: grid, scanlines, wireframe (3, the one chosen, by default) |
| `AURORA_DEMO_ARRIVAL=<folder>[,net=<ms>][,bw=<Mb/s>][,n=<objects>][,turn=<s>][,avatars=<n>][,worn=<m>][,pbr=1]` | Arrival test: objects, meshes and animated avatars all around, every asset downloaded from a local server |
| `AURORA_DEMO_HAIR=<layers>[,<segments>[,<empty>[,<period>]]]` | Transparent face load: stacked shells on the avatars |
| `AURORA_DEMO_HALO=1` | A wide translucent pane 3 m ahead of the start position, facing the camera (#556) |
| `AURORA_DEMO_BLEND_AO=1` | A face whose two halves differ in alpha mode, under a hat brim, ahead and left of the start position (#882) |
| `AURORA_DEMO_LASHES=1` | With `AURORA_DEMO_HAIR`: a red ball worn inside our hair, as eyelashes behind a fringe |
| `AURORA_DEMO_HAIR_CARDS=<n>[,layered[,outer]]` | Our avatar wears one face of `n` soft strand cards, overlapping in a shuffled order, or layered as a hair is made (#935) |
| `AURORA_DEMO_SHEER=1` | Our avatar wears a sheer knit dress over a red layer drawn after it (#1300) |
| `AURORA_DEMO_SOFT_MASK=1` | Our avatar wears a skin-tone Mask disc on the groin; soft fringe must stay visible (#1492) |
| `AURORA_DEMO_BUMP=tall`, `wide` or `sphere` | The normal-mapped test panel stretched 4:1, its normal map denser across or along it; or a low-poly ball with a baked normal map (tangent frame) |
| `AURORA_DEMO_MASK_ZERO=1` | The holed test panel with a Mask legacy material whose cutoff is 0: drawn whole |
| `AURORA_DEMO_RIGGED_PROP=1` | A rigged mesh chain rezzed on a cube ahead of the start position, and the same chain worn by our avatar |
| `AURORA_DEMO_FOLIAGE=1` | The same cut-out foliage panel at 5, 10, 20, 35 and 60 m along the start view (alpha coverage of the mips) |
| `AURORA_DEMO_PLANTS=1`, `far` or `dense` | Linden trees and grass: every species near or far along the start view, or a forest |
| `AURORA_DEMO_BLEND_SHADOWS=1` or `dense` | Alpha-blended panels in the sun (greenhouse pane, face and texture alpha), or 400 greenhouse panes |
| `AURORA_DEMO_FEET=opaque` or `blend` | A shoe on each foot of our avatar and of the crowd, and two blocks on the ground (contact shadows) |
| `AURORA_DEMO_LIGHTS=<n>` | Local light load: n small lights in the crowd |
| `AURORA_DEMO_PROBES=1` | Two manual reflection probes on the plaza: a box and a sphere |
| `AURORA_DEMO_GEOMETRY=<mb>[,<mb per second>]` | Geometry load: meshes filling the geometry arena |
| `AURORA_GPU_OOM=<mb>` | Video memory full: every growth of a GPU buffer or texture page of at least that size fails |
| `AURORA_DEMO_TURN=<degrees per second>[,<s turning>[,<s still>]]` | Fast camera turns, again and again |

### Details

- **`AURORA_DEMO_TEXTURES=<n>`**: n small cubes (9000 if the value is
  not a number), each with its own texture, of several sizes (including
  one that is not a power of two), sent first in low
  resolution then in full, as in a busy region.
- **`AURORA_DEMO_TEXTURES_CHURN=1`**: a third of the cubes removed at
  frame 300 (their textures evicted 2 s later: layers freed,
  page compaction), then back at frame 700 with new
  textures (freed slots reused, streaming again); capture
  after frame ~1000.
- **`AURORA_DEMO_CROWD=<n>`**: n more avatars (40 if the value is not
  a number) playing the idle animation, each wearing eight linksets
  of seven prims on bones all over the body (with `AURORA_DEMO_ANIMESH`,
  also the demo's rigged mesh), as in a busy shop.
  With `<worn>` (300 at most) and `AURORA_DEMO_ANIMESH`, a club
  crowd: each avatar dances (the whole body moves by half a metre,
  what it wears moves every frame) and also wears `<worn>` rigged
  meshes, of which one in a hundred has a touch script, transparent faces
  and `<triangles>` triangles (20,000 by default). This is the load of the
  search for the object under the cursor among dressed avatars:
  `AURORA_DEMO_CROWD=52,200` gives 13,500 objects of which 10,500 rigged meshes;
  combine with `AURORA_DEMO_HOVER=1`, `AURORA_DEMO_POS=129,134` and
  `AURORA_FPS_LIMIT=160` (whatever moves by more than 2 mm per frame near the
  cursor ray makes the search run again: the number of frames per second
  matters). With `,busy` after the number, what the sync still places
  itself in a place full of avatars is added: on each avatar a flexible
  ribbon and an ornament that spins with two linked prims, on our avatar
  a HUD of 250 prims (bottom left of the screen), and on the ground 60
  objects that spin with a linked prim. Combine with
  `AURORA_DEMO_STREAM=12000,400` for the motionless objects;
  `AURORA_SYNC_CHECK=1` gives what the sync visits, `AURORA_SYNC_SLICE=alt`
  compares with the old path.
- **`AURORA_DEMO_MATERIAL_ARRIVALS=<seconds>`** (2 if not a number): with
  `AURORA_DEMO_CROWD`, the linksets and the rigged meshes the crowd wears use a
  legacy material, and from 5 s on a new legacy material that no object uses
  arrives every `seconds`, as the materials of the avatars coming in arrive in
  a busy place. Combine with `AURORA_MATERIAL_REBUILD=alt`, `AURORA_PROFILE=1`,
  `AURORA_DEMO_ANIMESH=1` and `AURORA_DEMO_CROWD=52,100`: the old seconds rebuild
  every worn object (`rebuilt` about 50 a frame, `max: sync` about 7 ms), the
  new ones none.
- **`AURORA_DEMO_IMPOSTOR_FLIP=<seconds>`** (2 if not a number): the
  avatar limit alternates between its setting and one every `seconds`, so
  that the crowd's avatars go from impostor to drawn in full and back. With
  `AURORA_DEMO_CROWD=52,100`, `AURORA_DEMO_HAIR=3` and `AURORA_SYNC_CHECK=1`,
  the `impostor pictures` line must count no stale picture, centre or anchor
  (`AURORA_IMPOSTOR_POSES`).
- **Crowded avatar impostor capacity (#1579)**: Use `AURORA_DEMO_CROWD=120`, `AURORA_DEMO_ANIMESH=1` and
  `AURORA_DEMO_IMPOSTOR_FLIP=3`. Keep Fully rendered avatars at 16. Turn the
  camera through the crowd and back: visible avatars must retain a picture, or a
  neutral silhouette while awaiting their first capture, never disappear for lack
  of a tile. Offscreen avatars must not reserve slots needed on screen. Repeat
  with `AURORA_CPU_CULL=1`, and with Occlusion on and off. A silhouette may remain
  when more than 256 impostors are simultaneously visible (the 128 MiB atlas
  budget); increasing the full-avatar limit is not required. Capture a steady
  view and a view immediately after turning. Compare normal and flipped full
  rendering for attachment visibility. Human grid validation: repeat the failing
  busy-area camera motion at 16 fully rendered avatars, then at 100.

- **`AURORA_DEMO_HAIR=<layers>[,<segments>[,<empty>[,<period>]]]`**: our
  avatar and each avatar of `AURORA_DEMO_CROWD` wear `layers` rigged
  meshes (8 by default, 32 at most), each a shell around the head and
  one around the torso, a little larger from one layer to the next: the
  stacked strands of mesh hair and layered clothes, that is
  `layers` transparent fragments per covered pixel. `segments` (24 by
  default, 4 to 180) sets the triangles: 4 × segments² per layer and per
  avatar. The texture is made of vertical strands spaced `period` texels
  apart (16 by default, 4 to 256, out of 256), `empty` per cent of which are
  fully transparent (60 by default, 0 to 90).
  The inner layer is attached last, so that it is drawn first, as a hair
  made for the Legacy Viewer (`AURORA_RIGGED_BLEND`); the strands are 90 %
  opaque at the roots and 36 % at the tips: with `AURORA_RIGGED_BLEND=depth05`
  the outer layers look see-through (#935).
- **`AURORA_DEMO_HAIR_CARDS=<n>`** (1 to 400; with or without
  `AURORA_DEMO_HAIR`): our avatar also wears one rigged mesh whose single
  face holds `n` strand cards hanging around the back of the head, at
  several distances from it, in a shuffled order, both sides of each card;
  their strands are 95 % opaque in the middle and fade over many texels.
  The triangles of one face, not drawn back to front, as the Legacy Viewer
  leaves them after its vertex cache optimisation. With
  `AURORA_RIGGED_BLEND=copy` the cards leave shards where the head and the
  scene show through (#935, after #948); by default the hair is dense.
  `AURORA_DEMO_CAMERA=close,846,560,40` with `AURORA_DEMO_POS=133,129`
  frames them; capture at frame 780 (in a 1683 × 900 window,
  `close,840,610,40`).
  `<n>,layered`: the cards of a hair as it is made, listed from the head
  outwards, the inner ones dark blue and the outer ones near white, strands
  88 % opaque with a soft edge of three texels and nothing between them,
  drawn in the Legacy Viewer's triangle order like a mesh asset; the hair
  looks the same with every `AURORA_RIGGED_BLEND`. `<n>,layered,outer`:
  listed from the outside in: with `AURORA_RIGGED_BLEND=runs` the inner
  cards show over the outer ones (the hair turns dark blue, #935), by
  default and with `copy` they stay behind them.
  - To read in `perf summary`: `g_transparent` + `g_post` (see
    [Profiling](#profiling)), `blend`, `blend_tris_k`. `32` with
    `AURORA_DEMO_CROWD=40` and `AURORA_DEMO_POS="129,134"` gives ~970
    transparent faces (17 avatars drawn in full).
  - For the early discard of invisible fragments
    (`AURORA_BLEND_EARLY_DISCARD`): it only pays off where empty
    areas are in one piece on screen. `32,24,90,256` (large empty
    stretches) shows it, `32,24,60,16` (fine strands) gains nothing.
- **`AURORA_DEMO_SHEER=1`** (with or without `AURORA_DEMO_HAIR`): our
  avatar also wears two rigged shells around the torso, 1.5 cm apart: a
  white dress whose alpha wavers between 0.25 and 0.75 from one 4-texel
  stitch to the next (a sheer knit), and under it a red layer (95 %
  opaque), attached first on the same point so that it is drawn after the
  dress, as a body add-on worn under clothes (#1300). By default the dress
  hides the red layer everywhere and shows the body behind it, as the
  Legacy Viewer; with `AURORA_RIGGED_BLEND=split` the red layer shows
  through the dress in speckles, wherever its alpha is under one half. The
  default camera looks at our avatar's back.
- **`AURORA_DEMO_SOFT_MASK=1`** (with or without `AURORA_DEMO_HAIR`): our
  avatar wears a skin-tone disc on the groin (attach point 53), rigged to
  `mPelvis`. Its texture classifies as Mask (opaque centre, two-texel soft
  fringe). The Legacy Viewer never auto-masks rigged faces
  (`LLFace::canRenderAsMask`), so the fringe stays blended and visible;
  auto-Mask at cutoff 0.5 would punch it out. Look from below / the side:
  a solid peach disc with a soft edge, not a hard cutout or a missing
  attachment (#1492 / Physics Cock Max class).
- **`AURORA_DEMO_MASK_ZERO=1`**: the "Auto: mask" panel of the plaza (round
  holes of alpha 0) gets a Mask legacy material with a cutoff of 0 and the
  hover text "Mask (cutoff 0)". The Legacy Viewer keeps every texel at or
  above the cutoff (`alpha < minimum_alpha` in materialF.glsl and
  pbropaqueF.glsl), so the panel is drawn whole, its holes in the teal of
  the texture: creators use it to show opaque a texture whose alpha channel
  holds something else. Before the fix the holes stayed cut out.
- **`AURORA_DEMO_BUMP=tall|wide`**: the "Normal + specular" panel of the
  plaza becomes 0.45 × 1.8 m (`tall`) or 1.8 × 0.45 m (`wide`): its texture
  coordinates are four times denser one way. With `AURORA_TANGENTS=alt`, the
  relief of its bumps along the denser axis comes and goes (`tall` shows it
  best, the sun lighting the vertical tilts): about three times weaker with
  the former frame. Seen from `AURORA_DEMO_POS="133.5,128.6"`. `sphere`
  adds, in front of the panel, a low-poly mesh ball (12 × 8 quads, smooth
  normals, warped UVs) whose normal map is baked from a round, dimpled ball
  through the mesh's own MikkTSpace tangents, as a modelling tool bakes one
  (`demo_tangents.rs`). With the vertices' tangents (`AURORA_TANGENTS`
  default) it is round and evenly dimpled; with `pixel` or `derivative`
  each triangle turns the relief its own way and the facets show. Close-up
  from the start position: `AURORA_DEMO_CAMERA=close,652,455,30`, capture
  at frame 720; `AURORA_GRAPHICS_PRESET=low` leaves out the self-shadow line
  the sun draws across a low-poly ball.
- **`AURORA_DEMO_RIGGED_PROP=1`**: a five-link chain modelled down the
  back of the default skeleton, bound to the chest, and encoded as an
  upload does it (vertices normalized, the bind shape bringing them back
  onto the skeleton,
  `demo_rigged_prop.rs`), at frame 120. Rezzed on the dark cube 3 m ahead
  of the start position, it is a static mesh standing on the cube and
  filling its object box, as the Legacy Viewer draws a skinned mesh that is
  neither worn nor a playing animated object; a chain much smaller,
  floating above the cube, is its bind shape applied without a skeleton.
  The same mesh worn by our avatar hangs down its back.
- **`AURORA_BLEND_SHADOWS=0`**: alpha-blended faces cast sun and projector
  shadows only when rigged, as before. By default every blended face casts,
  as in LLPipeline::renderShadow (renderAlphaObjects): glass whose texture
  has opaque frames shadows its frames, blended foliage its leaves. Read at
  start: compare two launches, `AURORA_DEMO_BLEND_SHADOWS=1` for the look
  (with `AURORA_DEBUG_SHADOWS=1`), `dense` for the cost (`g_shadows_depth`
  and `sd0` to `sd2` of `perf summary`).
- **`AURORA_DEMO_FOLIAGE=1`**: a 2.5 m panel of thin blades and small
  `AURORA_ALPHA_COVERAGE=0` the 10 and 20 m panels thin out; with the
- **`AURORA_DEMO_PLANTS=1|near|far|dense`**: Linden trees and grass
  (`scene/plants.rs`), best from `AURORA_DEMO_POS="80,100"`. `1` (or
  `near`): the 21 tree species in three rows 26, 42 and 58 m along the
  start view, species 0 to 6 in the nearest row from left to right (the
  palm, species 3, straight ahead), and the 6 grass species in a row 8 m
  ahead; `far`: the same rows 30 m further and 30 m to the right, 56 to 88 m
  away (within the 96 m draw distance); `dense`: about 580 trees of every
  species, turned and sized at random, and 390 grass tufts over the land
  ahead (the cost of a forest, with `AURORA_PROFILE=1`). The species
  textures are not fetched: stand-ins drawn under their ids (bark on the
  left half, leaves in the top right quarter, grass blades from the bottom
  edge), only with this scenario. Close-ups: `AURORA_DEMO_CAMERA=close,790,185,80`
  on the tree behind the palm, capture at frame 760.
- **`AURORA_DEMO_BLEND_SHADOWS=1|dense`**: alpha-blended prims that cast
  sun shadows (`demo_blend_shadows.rs`), best from
  `AURORA_DEMO_POS="80,100"`. `1`: six 2 m panels lying 1.3 m above the
  ground, 14 m along the start view and to its right (clear of the
  first-launch windows), left to right: a greenhouse pane (opaque frame
  bars, panes at texture alpha 0.5), the same at face alpha 0.6, an opaque
  texture at face alpha 0.5, 0.03 and 0.95, and a texture at alpha 0.75.
  Their shadows on the ground, from left to right: the frame alone, solid;
  the frame alone, dithered; dithered; none; solid; dithered (the Legacy
  Viewer's rules, `fs_shadow` in `object.wgsl`). The classic sun lights
  with `min(n·l, shadow)` as the Legacy Viewer does, so a dithered shadow
  barely darkens ground the sun already lights at a slant: see the
  dithered ones with `AURORA_DEBUG_SHADOWS=1`. `dense`: 400 upright
  greenhouse panes over a 60 m square ahead, for the cost of the blended
  casters (`AURORA_PROFILE=1`, with `AURORA_BLEND_SHADOWS=0` to compare).
- **`AURORA_DEMO_FEET=opaque|blend`**: our avatar and each avatar of
  `AURORA_DEMO_CROWD` wear a box on each foot's attachment point, opaque or
  blended (color alpha 0.95: the many mesh bodies and shoes whose faces
  are blended though they look opaque), and two blocks stand on the ground
  left of the view from `AURORA_DEMO_POS="129,134"`, an opaque one and,
  further left, a blended one; nearer the camera, a hoof (a cylinder 14 cm
  wide) stands on the ground and a shoe hovers 2 cm above it, opaque or
  blended like the shoes. For the contact shadows
  (`AURORA_CONTACT_SHADOWS`): with `AURORA_DEBUG_SHADOWS=1`, the blended
  block takes them as the opaque one does; with
  `AURORA_CONTACT_SHADOWS=prepass` it does not (the blended faces were not
  in the depth the pass reads). Issue 988: with `AURORA_DEMO_SUN=0,12`,
  `AURORA_DEBUG_SHADOWS=1` and `AURORA_DEMO_CAMERA=close,615,672,50`
  (frame 780 at 60 fps), the ground under the hovering shoe takes its
  contact shadow; with `AURORA_CONTACT_SHADOWS=shell` it stays lit, a
  thin dark line along the sole's edge.
- **`AURORA_DEMO_LASHES=1`** (with `AURORA_DEMO_HAIR`, for example
  `1,48,20`): our avatar also wears a small red ball, a blended rigged
  mesh, inside the head shell on the side the default camera sees: the
  eyelashes behind a fringe of mesh hair, nearer the camera than the
  centre of the hair. The ball must not show through the strands; with
  `AURORA_RIGGED_BLEND=sorted` it does (the bug of before). Capture at frame
  620 and look at the back of the head.
- **`AURORA_DEMO_GEOMETRY=<mb>[,<mb per second>]`**: after the loading fade,
  meshes of 60,000 vertices keep entering the geometry arena until it
  holds `mb` megabytes of them (800 if the value is not a number), at 25 MB
  a second unless told otherwise (KB loads about 22 MB a second
  for 35 s). They are not drawn: the scenario only makes the arena outgrow
  its buffers, in the frames of the ordinary demo. The log gives each
  growth (`vertex arena grown to … MB`) and the end (`demo geometry: …
  meshes, … MB in the arena after … s`). With `AURORA_PROFILE=1`, read
  `max=` and `r_acquire` in `max:` on the seconds of a growth; with
  `AURORA_PROFILE_FRAMES=1`, the `render profile` line of the slow frame.
  What a growth costs is the first use of the new buffer, about 21 µs per
  MB (`aurora-render/src/arena.rs`). The arena's peak use is remembered
  for the region (`geometry-needs.txt` of the cache, that of the demo for
  the demo: `cache\demo`): the next run grows the arena at once behind
  the loading screen (`geometry arena grown ahead behind the loading
  screen` in the log) and no growth happens in the scene any more. Delete
  that file to measure a first visit again.
- **`AURORA_GPU_OOM=<mb>`** (demo or grid): every allocation of `mb`
  megabytes or more made through `aurora-render/src/gpu_alloc.rs` fails as
  if video memory were full (`0`: all of them): the geometry arena, the
  draw records, the culling tables, the joint palettes and skin bindings,
  the texture pages. With `AURORA_DEMO_GEOMETRY=700,60` and a value of 64,
  the vertex arena grows by smaller steps up to 63 MB, then the log says
  once `vertex arena could not grow past 63 MB (out of video memory
  (AURORA_GPU_OOM))` and the meshes left over are not drawn; the view keeps
  drawing and the log holds no `wgpu error` line. Without the variable, a
  buffer the driver could not make used to be installed anyway, invalid,
  and every write into it was an error, every frame.
- **`AURORA_DEMO_TURN=<degrees per second>[,<s turning>[,<s still>]]`**:
  once the loading screen is gone, our avatar and the camera behind it
  turn at that rate (180 if the value is not a number) for the first
  duration (1 s by default), then stay still for the second (3 s), and
  again: what comes into view has to be synced, listed and textured.
  Combine with a load scene (`AURORA_DEMO_ARRIVAL`, `AURORA_DEMO_CROWD`,
  `AURORA_DEMO_STREAM`) and `AURORA_PROFILE=1`, and read the worst frame
  of each second (`max=`, `max:`, `ft=`).
- **`AURORA_DEMO_LIGHTS=<n>`**: n small local lights (64 if the value
  is not a number, 256 at most), glowing spheres of 6 cm at
  head height, spaced 1.6 m apart on the crowd's plaza and beyond,
  with a 3 m radius (4.5 m once extended like Firestorm's deferred
  lights): the "face lights" of a crowd and the lamps of a club. The
  renderer takes 64 (`lights=` in `perf summary`): those that reach the
  view first, nearest first, then the nearest of the others. With
  `AURORA_DEMO_POS="129,134"` they are in front of the camera, 9 m away and
  more; with `"140,134"`, the camera is in the middle of them. Used to measure
  `AURORA_LIGHT_TILES`. With 150 and `"147,144"` (at night,
  `AURORA_DEMO_TOD=4`), the camera stands in the grid and about 100 of them
  reach the view: the list holds 64 of those (#1032: the 64 nearest all
  around the camera held only 50, and the pools of light halfway across
  the ground went out).
- **Probe selection (#446):** combine `AURORA_DEMO_PROBES=1` with
  `AURORA_DEMO_BUILD=move,0,0` and capture after frame 650. With no object
  selected, every inspector section must show "Select reflection probes"
  in the footer. Turn it on to select an invisible probe; turn it off to
  select the ordinary objects behind it. Check left/right clicks, face
  selection, a selection rectangle and Ctrl + wheel. The option starts off
  again in a new viewer session. Unit regressions use actual invisible CPU
  geometry and pointer clicks on the native inspector in all seven sections.
- **`AURORA_DEMO_PROBES=1`**: two manual reflection probes, as invisible
  prims on the plaza: a box of 10 × 8 × 6 m turned on itself, which the
  camera of `AURORA_DEMO_POS="135,134"` stands in, and a sphere of 12 m east
  of it. The automatic probes are kept out of the box, and mixed with the
  sphere's by its weight. Used to check `AURORA_PROBE_WALK` on the probes
  the plaza does not have otherwise (its only probe is the mirror's, which
  is a hero probe and no cube map).
- **`AURORA_DEMO_OVERDRAW=<n>`**: n rows of opaque walls (12 if the
  value is not a number, 40 at most) east of the plaza, one
  behind the other and each taller than the one in front, sent from the
  farthest to the nearest. Each wall sticks out above the next: occlusion
  keeps them all, and a scene pass that starts from an empty depth
  computes the colour of each in full before the nearer ones cover
  it. This is the worst case of overdraw, to measure
  what a prepass saves (`AURORA_MSAA_PREPASS`); a real region has
  less.
- **`AURORA_DEMO_REZ_EFFECT=1|loop|styles|cold`**: the rez effect (`scene/rez.rs`, Preferences ›
  Aurora › Rez effect), with `AURORA_DEMO_POS=129,134`,
  `AURORA_DEMO_CLOUD=1` (the avatars' loading state, which the grid
  replaces) and `AURORA_FPS_LIMIT=60` for frame-numbered captures
  (`demo_rez.rs`). Our avatar loads in the grid, then the line rises from
  its feet (frames 60–150). Four prims arrive at 300 whose textures come 150
  frames later: a glint passes over them at once (the line hiding nothing),
  each face is the grid until its own texture comes, then fades into it
  (455–480). Someone rezzes a four-prim table at 840, its
  texture a second later: the grid, then one line for the linkset. We rez a
  cube at 960 (FLAGS_CREATE_SELECTED) and copy it at 1020, both ready at
  once: a quick line, no grid. An avatar comes into view at 1080: the grid
  instead of the cloud while it loads, the line around 1265. Thirty boxes
  rezzed at once at 1380: six lines, the others show at once. A building at
  1560, its textures 1.5 s later: the grid, then a slower line. The avatar
  puts on a hat at 1640 (it rezzes on its head) and takes it off at 1760
  (its derez); the table is deleted at 1700 (the derez: the line comes down,
  then the grid fades out from the top, 1700–1742). The avatar changes
  outfit at 1820 (three items at once and new bakes, all 50 frames later):
  it keeps its old bakes meanwhile (only the new items are the grid), never
  the grid over it, then a glint around 1870 (capture 1830, 1868, 1892). It
  leaves at 1960 (its derez). The counts in the log at 2060 (`demo rez:`).
  `loop`: everything goes at once at 2160 (a burst: no derez) and the
  scenario starts again from 300, to watch it in one window.
  `AURORA_REZ_EFFECT=0` shows the Legacy look (grey, clouds, instant
  removal) for comparison, `AURORA_REZ_STYLE=2|3` the other looks,
  `AURORA_DEMO_TOD=4` the same at midnight. `styles`: the three looks side
  by side whatever the setting, a cube and behind it an avatar wearing two
  items under a label per look, arriving together every 9 s from frame
  300 (the grid traces in, waits 3 s, the line around frames 480–530),
  leaving at 720 (the derez) and back at 840.
  `cold`: a cold arrival, again and again (every 10 s from frame 300):
  400 objects of all sizes, 4 to 60 m away, arrive in four waves, their
  textures 0.1 to 4 s later (one in eight at once). With the effect on no
  face is ever grey: each face is the grid at once and fades into its
  texture (0.15 s) when it comes, at the same moment as without the
  effect; with `AURORA_REZ_EFFECT=0` (or Grey (Legacy))
  they are grey until their textures come, as before. The biggest on the
  screen get a glint as soon as they are built; a wall stands across the
  field's left half and nothing behind it gets one (`glint over` lines in
  the log, `behind a wall` in the counts). Capture frames 312, 330, 350,
  380 and 450 with each. The log tells each round's grids, lines, glints
  and faces out of the grid (`demo rez: round`).
- **`AURORA_DEMO_STREAM=<n>[,<wave>][,leave][,j2k=<folder>][,net=<ms>][,size=<m>]`**: n textured
  objects (2000 if the value is not a number) that arrive in waves after the
  loading fade, `wave` objects every 250 ms (100 by default, that is ~400 per
  second as on the grid; `wave` = n sends everything at once, like an
  arrival by teleport). Each object has its own shape and its own
  texture, at the sizes of a grid region (mostly 512 and 1024),
  decoded by the background tasks at a quarter of its size, then at full
  size 1.5 s later.
  - The log gives the end of loading (`demo stream: fully loaded in
    … s`, with the uploads prepared by the tasks / written by the main
    thread, and the texture pages created by the page thread /
    by the main thread, with their duration); with `AURORA_PROFILE=1`,
    read `stream`, `results` and the `s_*` parts in `max:` (`s_pages`:
    the pages created by the main thread).
  - With `,leave`, the region is left one second after loading,
    as by a teleport: all the objects are removed at once and
    their texture, which then holds the downloaded data of the size
    of its J2C file, is unused and to be evicted 2 s later (`demo
    stream: region left`, then `demo stream: textures evicted`); read
    `s_maintain`, `sync`, `s_sync_list` and `r_submit` in `max:`.
  - With `,j2k=<folder>` after the numbers, the textures are the
    `.j2c` files of that folder (a viewer's `cache\tex`, looped over
    if there are fewer of them than objects) instead of drawn checkerboards:
    each file is handed to the streaming as a successful read of the disk
    cache and follows the real path from the JPEG2000 decode onwards, at
    the level its on-screen size asks for. This is the return to a
    region already in cache, without the grid: read `demo stream: loaded in
    … s` then the `texture load:` line (see [Profiling](#profiling)).
    The folder is only read. Launched twice in a row, the scenario also
    replays the return with the cache of transcoded textures: the second
    time, the compressed textures come from the `.bc7` files left
    by the first run in the demo's cache (`cache\demo\tex`, to
    delete to start again from a first visit) and their J2C file
    is requested only if a finer level is needed.
  - With `,net=<ms>` after `j2k=`, the files are downloaded instead of
    being handed over: a local asset server (`demo_asset_server.rs`)
    answers each request after `ms` milliseconds, the round trip to a
    distant grid (200 for Europe to Agni), and the textures follow the
    whole path of a region seen for the first time: the fetcher with its
    queue and its connections, the first range, the ranges that follow,
    the decodes. Their ids change at each run, so the disk cache never
    holds them (their files are left in `cache\demo\tex`, like any
    downloaded texture). The log tells when half, nine tenths and all of the
    textures showed their first pixels (`demo stream: … of … textures
    shown after … s`), then `demo stream: loaded in … s` when nothing is
    left to download or decode; the `texture load:` line gives the
    requests, the bytes and the wait before the first pixels. Use `wave` =
    n: the measure is that of an arrival.
  - With `,size=<m>`, the objects are that large (0.5 m by default) and
    spaced accordingly: their textures cover more of the screen and ask
    for finer levels, as the walls and floors of a region do. 400 objects
    of 3 m give levels close to those of a grid arrival.
- **`AURORA_DEMO_ARRIVAL=<folder>[,net=<ms>][,bw=<Mb/s>][,n=<objects>][,turn=<s>][,avatars=<n>][,worn=<m>][,pbr=1]`**:
  an arrival in a busy region, to measure what the loading shows first.
  `n` objects (1,200 by default) arrive all around our avatar, the nearest
  first and 600 a second, from 3 to 120 m away and larger with the
  distance; one in three is a mesh and each has its own texture. `avatars`
  avatars (16 by default) stand nearby, each playing two downloaded
  animations over the demo's standing one; with `worn`, each wears that
  many rigged meshes (those of the folder's `.mesh` files that have a
  skin), each with its texture: `avatars=30,worn=6` is a busy place. With
  `pbr=1`, one object in twelve, a prim, wears a PBR material (the `.mat` files of
  the folder's `mat` subfolder), whose textures are served from `tex`
  when it holds them, else one of the others in their place. Every asset is
  downloaded from the local asset server (`demo_asset_server.rs`, bound to
  this machine only), which answers each request after `net` milliseconds
  (200 by default; about 70 for the asset hosts on a good link); with
  `bw=`, the bodies of all its connections share that many megabits a
  second, one after the other, as on a link held by its throughput (about
  100 on a home link measured on Agni, where 24 requests at a time fill
  it): without it only the round trip limits the downloads. The files
  are those of the `tex` (`.j2c`), `mesh` (`.mesh`, static ones only) and
  `anim` (`.anim`) subfolders of `<folder>`, only read, under ids proper
  to the run so that no disk cache holds them (the run's files are left
  in `cache\demo`, named `dea221a0-…`). `turn` seconds after the first
  objects (3 by default) the avatar turns round: what was behind is now in
  front, and the loading must follow.
  - The log tells when half, nine tenths and all of what is in front after
    the turn showed, weighted by its size on screen (`demo arrival: front
    50% shown after … s`; an object counts once its geometry is built and
    its texture shows its first pixels), the same for everything around
    (`demo arrival: all …`), when the animations arrived (`demo arrival:
    animations …`), when what the avatars wear showed (`demo arrival: worn
    …`, built and textured), then the requests and megabytes served (`demo arrival:
    done`). The `texture load:` line gives the decodes and their CPU.
  - A run lasts about 20 s at 200 ms; to compare two builds, alternate
    their runs (see [Profiling](#profiling)).

## Interface

| Variable | Effect |
|---|---|
| `AURORA_DEMO_TOOLBAR=1` | Open the "Toolbar buttons" window; the bars mockup's custom state, side toolbars, labels. |
| `AURORA_DEMO_AREA_SEARCH=1\|filters\|ranges\|scope\|fields\|columns\|all-columns\|link\|parcel\|sale\|search\|menu` | Open Area Search with named objects and sale entries; `menu` opens its right-click menu. |
| `AURORA_DEMO_TEXTURE_LIBRARY=<mode>` | Compact inventory library with synthetic folders, eight types and thumbnails. |

`AURORA_DEMO_AREA_SEARCH`: `1` opens the object list; `parcel` restricts it to
the agent's parcel, `sale` shows sale entries, `filters` opens Filters (required/excluded properties, permissions, click action and scope), and
`search` searches the localized fountain name, and `menu` right-clicks the
second row to open the result menu. `fields` expands the six-field Search form,
`ranges` opens distance and sale price, `scope` is an alias for Filters, `columns` opens the Columns popout, `all-columns` shows all ten result columns,
and `link` round-trips a fountain search through the SLapp parser.
Capture after frame 740 in English, French and Japanese, at normal and 150% UI scale. Check the column
sorting, combined text and flag filters, the in-field crosses that clear only
their filled text filter and return keyboard focus to it, Reset, Refresh preserving filters,
selection, Inspect, Zoom/double click, Copy UUID, and the toolbar button reopening
a minimized window. The right-click menu (`menu`): Ctrl and Shift clicks select
several rows and every entry acts on all of them; Sit, Teleport to and Buy need
one row (Buy one for sale), Zoom one within the draw distance; a right click
outside the selection selects that row alone; Select all, Clear selection and
Filter my objects (the owner field, opened). Edit, Delete (with its question),
Return, Inspect, Touch, Derender & blacklist and Sit work on the demo objects. On the grid, compare the received roots with the Legacy
Viewer, check parcel boundaries and neighboring regions, linked prim names,
group-owned objects, late name replies, derez/movement and region changes.
Check all six text fields and whole-field regex (including invalid patterns),
owner permission masks, required/excluded flags, min/max price and precise
distance, each click action and current parcel. Every edit updates results immediately
without hiding the table. Results, Search, Filters and Ranges are left-aligned tabs;
the table remains below every selected form. Results hides the form to give
the table more room. Columns
remains a right-aligned popout. Reset at the bottom right clears the current section's
settings; the table's Reset clears all search fields and filter options. Save as
default persists options without search text. Column visibility survives
reopening/restarting; LI and prim count sort numerically. Copy as link restores
the search rules when clicked in chat. Include attachments with child prims
excluded: attachment roots must still appear, and their children only when
enabled. Grid validation is performed by the human, without sharing credentials.
Keep a Build/Inspect selection while requests are arriving: it must stay selected.
Objects outside the viewer's received scene cannot be enumerated by this window.

`AURORA_DEMO_TOOLBAR`: `1` opens the "Toolbar buttons" window at frame 300;
`drag` holds Friends over the toolbar with an insertion marker (the empty
side toolbars show where a button can go), `add` releases it, `reorder`
moves Preferences beside Inventory, and `menu` opens a button's context
menu. Capture after frame 670. Use an unmodified demo toolbar with Friends
absent for the add/drag scenarios. Changes use the demo's separate saved
settings. The bars mockup's states, for this run only (nothing is saved):
`custom` (Environments, Radar and Teleport history in the left toolbar,
Performance out of the dock, the window open and, from frame 652, Build
dragged from it between the mini-map and Sit), `side` (the same toolbars
without the window; `sidemenu`: Radar's right click at frame 650, the
vertical toolbar's menu), `right` (them in the right toolbar), `labels` (the
dock in icons and labels, the left toolbar in labels). Capture at frame 780.

`AURORA_DEMO_TEXTURE_LIBRARY`: `preview-controls` opens a 620 × 720 texture inspector and material
editor side by side (use `AURORA_WINDOW_SIZE=1600,1000`, capture after frame 740).
The material has a dirty, isolated draft; test its parameters, map pickers, Save,
Save As and Cancel, and close it with unsaved changes. `preview-controls-stretch`
switches aspect preservation off for the image. Resize either window and check
that the material window stays at its released size for several seconds, including
with narrow wrapped controls. Its shape selector offers the importer's sphere,
cube and Aurora model, with independent orbit/reset controls. The
`preview-controls-cube` and `preview-controls-aurora` variants select those models
for offline captures. `preview-controls-drop` drags the texture inspector's image
onto the material's empty Metallic-roughness map at frames 760–765 (capture at 763 to see the outline
over the swatch, after 770 for the replaced map and its name). All four assigned maps must show their pixels in the large
map thumbnails, including textures already resident in the world. Check
the image fitting, native 100% scroll view, ratio presets/custom ratio, dimensions,
description, UUID, refresh and permission-gated PNG/TGA export. Local map uploads
open the texture import inspector to review and confirm their fee before replacing
the map. These modes are offline; inventory save/copy acknowledgements are simulated.
`vertical` and `horizontal` open the corresponding presentation at frame 600;
`types` and `types-horizontal` also open the checked
type menu at frame 650 for screenshots; `inventory` also reveals the actual library folder
in inventory, `search` filters stone PBR materials (including a nested polished
folder), and `empty` keeps the sample resources outside the empty library. `drop` performs a real card drag onto the
modifiable demo surface at (480, 430), for a 1280 × 800 window. Library scenarios
make demo objects modifiable so manual surface drops can also be exercised.
`objects` and `objects-horizontal` show the Objects filter, with a thumbnail,
single-object and coalesced-object fallback icons, and a no-copy group with an image.
The synthetic inventory uses `#Aurora/Aurora Library` with eight conventional
folders and mixed actual types in nested folders. The five new types include
empty document/player fixtures; real asset playback and teleport need human
grid validation.
`objects-inventory` also reveals actual object rows in inventory: the group
must use the same multi-cube icon as its library card. The folder picker lists
the unified root and all real descendants; organizing reveals Aurora Library.
Click the type selector twice to open then close it. Check several types and
combine them with search/folder filters. The type selector only uses multiple checks.
`objects-drop` drags the first object into the world at frame 650 through the
inventory rez path, using a projected ground point and an elevated camera
unless `AURORA_DEMO_CAM` is set (capture at frame 800). The offline simulator stands in with
a cube; actual object/coalesced geometry and server rez permissions need grid validation.
`objects-nocopy-drop` drops the no-copy group instead: its card disappears after
the drop, and the stand-in cube appears in the world.
Library scenarios keep the unrelated AO window closed from startup.
`baseline` keeps the library closed in the same scene for a profiling comparison.
`preview` double-clicks an actual inventory texture and leaves its preview open;
`preview-drag` then drags the preview image onto that surface, and `inventory-drag`
drags the actual inventory row directly. Both use frame 720 for dragging.
`apply` / `apply-pbr` click a texture / PBR card at frame 650; combine with
`AURORA_DEMO_BUILD=face,480,430` and `AURORA_DEMO_BUILD_TAB=texture:classic`
(or `texture:pbr`) to select that face first. Classic is before PBR in the build
window. The click must update only selected editable faces, preserving other
faces, transforms and selection; starting a drag must not apply to the selection.
`material-preview` double-clicks a material card and shows its 3D preview with
an explicit Drag handle. `material-preview-rotate` left-drags the scene at frame
700 without starting inventory cargo; `material-preview-drag` also drags the
handle onto the surface at frame 760. Check that rotation changes only the
preview camera, that the handle does not rotate the scene, and that dropping
it applies the material. Its permission/retention checks match other sources.
`multi-preview` opens two textures and two materials in separate resizable
windows at frames 620–680, then reopens the first texture at frame 720: four
windows remain, with no replacement or duplicate. `multi-preview-closed` also
closes the library at frame 760; all four previews must remain visible. Check
individual close/minimize/restore, independent material cameras, texture drags
and material handles, including from inventory outside the library. Reopening
the same item restores/raises its existing preview without resetting its view.
`preview-sizes` manually shrinks the first texture at frame 640, opens another
texture at 670, resizes the first material at 710 and opens another material at
740. At 780, the four windows are arranged for comparison: each pair must have
the same size, with distinct stored texture/material sizes. Resizing an older
window must affect only subsequent opens of that type. Minimizing, restoring,
moving windows or automatically fitting a smaller viewport must not replace
the preferred sizes; both preferences survive restart.
`cascade`, `cascade-material`, `cascade-notecard` and `cascade-script` open six
real inventory windows of the selected type at frames 620–720 without forcing
their geometry. New instances advance by 32 logical pixels right/down from the
last created visible, non-minimized instance of that type; moving or selecting
an older window does not change that order. Script editors from inventory and
objects share one group. Check fitting at screen edges, minimizing/closing the
last instance, reopening an existing item, and independent editable notecard
drafts. Texture and material sizes retain their separate preferences.
`inventory-move`, `inventory-move-hover` and `inventory-move-rejected` open
the real library subtree in a separate inventory window. Actual pointer events
select the first two wood textures (Ctrl-click), start dragging one selected
row at frame 660, hover the stone folder at 670 and release at 740. Check the
two-item badge, Move cursor, target outline, one-second spring opening, and
that both actual inventory items change folders after the acknowledgement.
`inventory-move-hover` holds until 840 (capture at 800); `inventory-move-rejected`
protects the destination and must keep both items in their original folder,
showing rejection instead of moving them into the enclosing library root.
Also test Shift selection, moving between inventory windows, folder/child
selections, same-parent no-ops, Escape, no-copy items, and protection changes
while hovering. Single-item world/preview drags continue to work.
`inventory-move-scroll` adds 81 folders, drags the same selection below the
viewport at frame 670, returns to the previously hidden last folder at 950 and
drops at 1050 (capture at 1120). Check automatic downward scrolling and both
textures arriving together. Also drag above the viewport to scroll upward and
return to its middle to stop. This must work in filtered and partial inventories.
`inventory-move-fold` hovers the stone folder until it opens, leaves at 760,
then returns at 780 until it closes (capture at 750 for open, 900 for closed).
An uninterrupted hover toggles only once; leaving/re-entering permits another
toggle. Source folders can close without losing the dragged selection.
No-copy resources cannot be applied to multiple selected objects. Restricted
resources wait for object contents; click again after the loading message.
Capture after frame 780 to allow previews to
load. All inventory/assets are synthetic; no grid or credentials are used.
Check the header's presentation switch, Ctrl+Space and bottom toolbar button,
folder/type/search filters, double-click preview and the inventory management
button. Move or copy actual textures, materials and folders/subfolders into
`#Aurora/Textures Library` through inventory to check automatic refresh. Item
and folder links must not appear, fetch their targets or contribute resources.
Removing or moving actual content out of the folder removes it from the library.
On a modifiable demo object, drag to one face or Shift-drag to all faces;
Ctrl-drag inserts into contents. Alt-drag a texture onto a PBR face restores
legacy texturing. Grid validation by the human must cover real asset fetches,
ModifyMaterialParams/ObjectImage acknowledgement and no-copy/no-transfer rights.

The values of these switches are English words. The French words they had
before (`amis`, `groupes`, `repere`, `historique`…) are still accepted.

| Variable | Effect |
|---|---|
| `AURORA_DEMO_WINDOWS=<mode>` | Collapse, move and restore of the inventory title bar |
| `AURORA_DEMO_WINDOW_EDGES=<l\|r\|b…>` | Screen edges the windows may cross for the run |
| `AURORA_DEMO_MINIMIZED=<mode>` | Minimized Conversations: short bar, unread outline, expanded by its toolbar button |
| `AURORA_DEMO_SNAP=<mode>` | Window snapping: Alt attachment, free move, detachment, grouped move, resize |
| `AURORA_DEMO_RESIZE=<mode>[,<window>]` | Resize marks of a window: corner or edge hovered, corner or edge dragged or swept in and out |
| `AURORA_DEMO_EXIT=<mode>` | Exit modal and offline choices |
| `AURORA_DEMO_UI=<tab>` | Opens People (tab), the inventory and the chat |
| `AURORA_DEMO_RADAR=1`, `tp`, `walk`, `friends`, `groups` or `profile` | People window: radar, radar after a teleport, a resident who comes and goes, Friends tab, Groups tab, a group's profile |
| `AURORA_DEMO_PEOPLE=<mode>[:width]` | People scenarios, including nearby/friends lists and contact-set visual editors |
| `AURORA_DEMO_PEOPLE_FILTER=<text>` | Initial name filter for a People demo scenario |
| `AURORA_DEMO_ANTI_SPAM=1` | Synthetic offer, script prompt and sound floods with Protections controls |
| `AURORA_DEMO_RLVA=enabled` | Scripted restrictions and the global opt-in in Preferences > RLVa |
| `AURORA_DEMO_OPTIONS=<page>[/<tab>][:<what>]` | Opens preferences on a page, search, modified settings, open cogs, reset, or a graphics profile dialog/list |
| `AURORA_DEMO_GRIDS=1\|pages` | Opens the grid manager with synthetic metadata |
| `AURORA_DEMO_TOOLBOX=1`, `edit` or `<target>:<tool>` | Opens the Dev Toolbox (Customize, or a target and a tool pressed) |
| `AURORA_DEMO_VOICE_PREFS=<modes>` | Voice preference overrides for offline comparisons |
| `AURORA_DEMO_COLORBLIND=protan\|deutan\|tritan\|mono[:strength]` | Colour blindness of this run (Preferences › Accessibility), never saved |
| `AURORA_DEMO_CONTRAST=1\|0` | High contrast palette on or off for this run (Preferences › Accessibility), never saved |
| `AURORA_DEMO_COLORS=accent:<hex>,surface:<hex>,text:<hex>` | Interface colours of this run (Preferences › Appearance › Skin), never saved |
| `AURORA_DEMO_PERF=compact`, `full`, `network` or `monitor-<section>` | Opens the Performance window: compact, the resource monitor, or one of its sections |
| `AURORA_DEMO_BUY_CURRENCY=<mode>` | Demo balance and "Buy L$" window |
| `AURORA_DEMO_BUY_LAND=<mode>[:confirm]` | Plaza parcel for sale and "Buy Land" window |
| `AURORA_DEMO_BUY_PASS=sell\|none\|own` | The ground menu's "Buy access" on the plaza parcel |
| `AURORA_DEMO_PAY=<mode>` | Demo balance and the Pay window of a resident or a group |
| `AURORA_DEMO_MONEY_TRACKER=<mode>` | The money tracker on a session of payments, in either look |
| `AURORA_DEMO_PAYMENT=card\|legacy\|sent\|sent-open\|friend\|off` | A direct payment as a card in the conversation with the other resident |
| `AURORA_DEMO_LOGIN=<mode>` | Offline login screen and account carousel |
| `AURORA_DEMO_LOGIN_SUBMIT=<frame>` | Press Log in of the open login form at that frame |
| `AURORA_DEMO_LOGIN_WAIT=logout\|sudden\|refused` | A previous session of the demo account still on the grid: the login's countdown |
| `AURORA_NEWS_URL=<url>` | Login news read from somewhere other than `main` |
| `AURORA_DEMO_CRASH=<mode>` | Offline crash and recovery window, a frozen viewer ended from outside, out of memory, stack overflow or a critical thread's death |
| `AURORA_DEMO_CRASH_DIR=<folder>` | Isolates the crash reports of offline tests |
| `AURORA_DEMO_CRASH_SEND=<options>` | Crash report sending with a simulated GitHub |
| `AURORA_CRASH_CAPTURE=<file.png>` | Screenshot of the recovery window in demo |
| `AURORA_CRASH_CAPTURE_EXIT=1` | Closes the window after its screenshot in demo |
| `AURORA_DEMO_UPDATE=<mode>` | Interactive update mock-up on the login screen |
| `AURORA_DEMO_GPU_DRIVER=old\|old-linux\|aged` | Out of date graphics driver warning on the login screen |
| `AURORA_DEMO_GPU_STALL=<frame>[:<seconds>]` | The render thread holds a frame as a graphics card that stopped answering would |
| `AURORA_DEMO_GPU_LOST=<frame>` | The GPU device is taken as lost at that frame, as after a driver reset |
| `AURORA_DEMO_OVERLAY_HOOK=<file>` | An overlay hook DLL counts as loaded in the viewer (Discord, Steam…) |
| `AURORA_DEMO_SAFE_GRAPHICS=offer\|active` | The safe graphics card on the login screen |
| `AURORA_DEMO_UPDATE_MAINTENANCE=<mode>` | Announces a cache invalidation and/or a reset |
| `AURORA_UPDATE_TEST_ROOT=<folder>` | Throwaway installation served by a local feed |
| `AURORA_UPDATE_TEST_VERSION=<tag>` | Dev build with a synthetic version for this test |
| `AURORA_DEMO_BUG_REPORT=<mode>` | Public bug report with demo data, nothing sent (no effect while the form is disabled) |
| `AURORA_DEMO_REPORT_ABUSE=<mode>` | Report Abuse window, its checks and its sending, nothing sent |
| `AURORA_DEMO_MAP=1`, `mini` or words among `search:<text>`, `drawer:<tab>`, `landmarks`, `friend`, `legacy`, `coordinates`, `beacon[-click|-offscreen]` | Maps, search, drawer, landmarks, followed friend, side panel layout, coordinate entry and clickable 3D beacon |
| `AURORA_DEMO_NOTIF=1`, `AURORA_DEMO_STATUSMENU=1` or `invisible`, `AURORA_DEMO_MICMENU=1`, `AURORA_DEMO_NAVEDIT=1` | Notification list, status menu, microphone panel, location field |
| `AURORA_DEMO_PARCEL_NAME=<text>` | Any parcel name in the place island (odd names, line breaks) |
| `AURORA_DEMO_BARS=<words>` | Top and bottom bars: islands and dock, Legacy bars, place island, favourites, menus, balance, clock, hidden interface, immersive mode, chat autohide; `profiles` / `fpspresets`: saved graphics fixtures / FPS hover card |
| `AURORA_DEMO_TOASTS=1\|bell\|click\|minimized` | Friend online/offline and IM toasts, unread pill, Conversations button flash, click on an IM card |
| `AURORA_DEMO_CONV_BUTTON=1\|mute\|still\|menu` | The Conversations button flashing for nearby chat and an IM, nearby chat left out, Reduce motion, its right-click menu |
| `AURORA_DEMO_ATTENTION=1\|front\|capture` | Taskbar flashes and system notifications for messages, mentions and offers arriving in the background |
| `AURORA_DEMO_MENTION=list\|names` | The chat bar's `@` list of residents to mention, or the prediction of a nearby resident's name |
| `AURORA_DEMO_KEYWORDS=1\|prefs` | Keyword alerts on for the run: nearby chat, IM, group chat and object lines holding a keyword, coloured and with their sound; or the preferences' settings, the lines sent again and again |
| `AURORA_DEMO_NOTIFS=1\|dnd\|list\|history\|legacy\|legacy-history\|object\|object-own\|object-group\|object-history\|legacy-object\|group-notice\|legacy-group-notice\|group-notice-saved` | Notification center: cards, burst, paused bell, history, Legacy toasts, object inventory offers and group notices |
| `AURORA_DEMO_DELIVERY=1\|several\|open\|history` | A Marketplace delivery arriving in Received items: its card under the bell, several at once, "Show in inventory", its history entry |
| `AURORA_DEMO_PRESENCE=<place>` | Friends' online / offline toasts at one of their six places, waiting under the bell's cards and list |
| `AURORA_DEMO_FRIEND_PROMPT=1\|later` | Friendship offers in the middle of the screen (#465), one waiting behind the other |
| `AURORA_DEMO_REGION_MESSAGE=1` | Messages to the whole region in the middle of the screen (#1404), one waiting behind the other |
| `AURORA_DEMO_SIM_VERSION=1\|on` | The region's simulator version changes during the run: no notice by default, one with `on` |
| `AURORA_DEMO_OBJECT_IM=off\|region\|parcel\|strict` | Objects' messages from near and far, with the filter of Preferences › Privacy set for this run |
| `AURORA_DEMO_ABOUT=info\|licences[:end]` | Help › About Aurora Viewer opens on its Info or Licences tab |
| `AURORA_DEMO_COLLISIONS=1\|chat` | Residents bump, push and hit the agent; the "Bumps, pushes & hits" window opens on the list |
| `AURORA_DEMO_RESTART=<seconds>[:manager][:fast][:still][:min][:lm]…` | The demo region announces its restart: the alert card, its steps, pill, bell card, Firestorm's box |
| `AURORA_DEMO_LINK=stall:<frame>[:<seconds>],lost:<frame>` | The region goes silent (the unsteady-connection pill and its count), or the link is lost (login screen offering to go back to the same place) |
| `AURORA_DEMO_COLORPICK=1` | Opens the compact colour picker of the first swatch shown |
| `AURORA_DEMO_CONV=1`, `AURORA_DEMO_TALK=1` | Group conversation, open microphone |
| `AURORA_DEMO_CHAT_LINES=1\|legacy\|volumes` | Nearby chat lines above the chat bar: cards by sender, or the one-line look |
| `AURORA_DEMO_OBJECT_CHAT=aurora\|compact\|bubbles\|legacy\|lines[:on][:im][:imlines]` | Residents talking among objects in the nearby chat, in a style; `on` makes what no person sent compact and quiet; `im` the friend's conversation, `imlines` its system lines compact |
| `AURORA_DEMO_TYPING=1` | We type in the nearby chat from our arrival on: typing animation, dots on our plate |
| `AURORA_DEMO_CONV=bubbles[:busy\|group\|local\|legacy]` | Conversations as a messenger: bubbles, head with picture and status; or the legacy lines |
| `AURORA_DEMO_CONV=aurora[:flag…]` | Conversations in Aurora's workspace (column, head, grouped messages, members, calls), roomy or compact |
| `AURORA_DEMO_LURE=<case>` | Teleport offers and requests received |
| `AURORA_DEMO_MATURITY=agree\|disagree\|fail` | Content rating changed, the server's answer |
| `AURORA_DEMO_HOME=<outcome>[:set]` | World › Set Home to Here and the region's answer |
| `AURORA_DEMO_USERINFO=hidden\|liaison\|opensim\|fail\|savefail` | Preferences › Privacy › Visibility, the account settings the grid keeps |
| `AURORA_DEMO_SLURLS=1\|offer` | Place links with their region rating badges, one region looked up |
| `AURORA_DEMO_APPLINKS=1\|actions\|preview\|<command>[:restore]` | URI examples, visible action markers, their previews or a minimize/reopen scenario |
| `AURORA_DEMO_EXPERIENCE=1\|auto\|profile\|blocked\|walk\|arrive` | Experiences asking: a door to click, a pad to walk into, a parcel greeter on arrival |
| `AURORA_DEMO_CONTACTS=<tab>` | Contacts tab of Conversations |
| `AURORA_DEMO_CHATLOG=<mode>` | Conversation history: files, search, History window, offline IMs |
| `AURORA_DEMO_GLYPHS=1\|2\|rtl\|rtl2` | Names, titles and chat in extended characters (system fonts); `rtl`, `rtl2`: right-to-left text |
| `AURORA_DEMO_EMOJI_PICKER=1\|<text>` | Conversations open on the emoji picker, a text in the input field |
| `AURORA_DEMO_EMOJI_CODES=<text>` | Conversations input holding a text with the keyboard: emoji code suggestions |
| `AURORA_DEMO_CHAT_INPUT=1\|long` | Paragraph drafts in the conversation and nearby-chat inputs |
| `AURORA_DEMO_SPELLING=1\|menu\|replace` | Misspelled words underlined in the chat bar and Conversations; their suggestions; auto-replace while typing |
| `AURORA_DEMO_CONV_EMOJI=1` | Messages of emoji alone in the friend's conversation, drawn large (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_CONV_PREVIEWS=1\|states\|videos\|clips\|services\|links\|net` | Picture, GIF and YouTube cards; `clips` shows video files and an animated WebP; `links[:gif\|image\|video]` isolates URL actions; `services[:gyazo,…]` shows service pages and social posts; `net` uses real sites |
| `AURORA_DEMO_PREVIEW_PLAY=card\|window` | The first video card plays by itself, in its card or in the player window |
| `AURORA_DEMO_VIDEO_VOLUME=<0-100>` | The level a playing video starts at, as if kept from an earlier one |
| `AURORA_DEMO_GIFS=1\|klipy\|sent\|menu\|nokey\|net` | Conversations open on the GIF picker of the field, its results made here; `sent` sends the first GIF; `menu` the button's right-click menu; `nokey` no button; `net` the real services |
| `AURORA_DEMO_PREVIEW_ZOOM=1\|gif\|zoom\|pan\|window\|copy\|links` | Enlarge a picture, open its independent window, click Copy image or hover its URL actions |
| `AURORA_DEMO_CONV_SELECT=all\|part` | Conversations open with its lines selected (Ctrl+A, or across three lines) |
| `AURORA_DEMO_CONV_MENU=local\|im\|group\|conference` | Right-click menu of a conversation of the list open (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_FRIEND_LOCATION=allowed\|denied\|offline\|altitude\|join-map\|join-contact\|join-fly\|join-fly-blocked\|join-fly-late\|join-fly-custom` | Remote friend's received map rights in contact, conversation and profile actions |
| `AURORA_DEMO_CONV_POPOUT=1\|option\|nearby\|nearby-out\|dock\|dock-hold` | Detached conversations and title-bar docking (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_CONV_REORDER=1\|hold\|out\|nearby` | Drag the first conversation after the next of its category, or hold it over its gap, or drag it (or the nearby chat) out of the window (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_CONV_GIVE=away\|over\|many\|drop\|give` | Inventory carried to the IM shown: drop feedback, give confirmation, or give and its notice (all conversation styles) |
| `AURORA_DEMO_FLASH=1` | Tess writes every 8 s: her conversation's entry flashes while another is shown |
| `AURORA_DEMO_THUMBNAILS=bar\|top\|off` | The conversations' thumbnails beside the chat bar, at the top right or hidden, for the run (#1134) |
| `AURORA_DEMO_SNOOZE=<seconds>[,auto][,ask]` | Tess writes in the group chat every 6 s; its snooze lasts `<seconds>`; `auto` snoozes it at the 14th second, `ask` asks the duration first |
| `AURORA_DEMO_CONFERENCE=1\|<seconds>` | A conference invitation every 20 s (or `<seconds>`), from a stranger then a friend in turn |
| `AURORA_DEMO_CONV_SCROLL=up\|new` | The friend's conversation, long, shown from its top: the button back to the latest messages; `new` with three messages come in (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_CONV_LINE_MENU=1` | Open a visible message's context menu in Aurora or Compact (with `AURORA_DEMO_CONV`) |
| `AURORA_DEMO_PROFILE=<avatar>[:tab\|edit\|compact\|form\|publish]` | An avatar's profile window |
| `AURORA_DEMO_CONTENTS_DROP=world\|panel[:ctrl][:drop]` | A selection of four inventory entries carried over an object or onto its Contents tab, with or without Ctrl |
| `AURORA_DEMO_GIVE=profile\|avatar[:over\|:drop]` | Inventory carried to a resident's profile window or to their avatar in the world: hint, outline, give confirmation (#991) |
| `AURORA_DEMO_FEED=<url>` | Profile "Feed" tab on this page |
| `AURORA_DEMO_APPEARANCE=<mode>` | Appearance window and outfits |
| `AURORA_DEMO_OUTFITS=<count>[,closed][,scroll][,locate]` | A large wardrobe in the Appearance demo: generated outfits, the window closed, scrolled or located on the current outfit |
| `AURORA_DEMO_OUTFIT_RESTORE=warm\|cold` | The outfit's attachments restored at login against a region that puts part of it back by itself |
| `AURORA_DEMO_SHAPE=<mode>` | Worn shape editor and permission scenarios |
| `AURORA_DEMO_WEARABLE=<type>[:mode]` | Skin, hair, eyes and system clothing editors |
| `AURORA_DEMO_PHYSICS_EDIT=1\|new\|drag` | Physics layer editor: a worn layer, "New physics", sliders moving |
| `AURORA_DEMO_AVATAR_HEALTH=undeform\|refresh` | Me › Avatar health's Undeform avatar or Refresh attachments at frame 600 |
| `AURORA_DEMO_INVENTORY=<mode>` | Synthetic inventory, animation playback previews, link type icons, lost inventory recovery, menus, layout notecards, filters, drag feedback, windows, keyboard, search selection and advanced search; `aurora…` modes show Aurora's window at each width |
| `AURORA_DEMO_GESTURES=manager\|editor\|numpad\|play\|replay\|channel\|completion\|completion-send\|list\|communicate\|communicate-open` | Gesture manager, editor, numeric shortcuts, playback, back-to-back replays, commands to worn scripts, completion, the chat bar's list of gestures and menu access |
| `AURORA_DEMO_IMPORTS=<mode>` | Import window and synthetic assets; `launcher` / `settings` open empty tabs; `icon-bar` opens the quick import bar; `type-*` opens independent type windows |
| `AURORA_DEMO_IMPORT_FILE=<path>` | Local asset to open in the demo flow |
| `AURORA_DEMO_IMPORT_REGISTRY=<file>` | Isolated persistent library for the import tests |
| `AURORA_DEMO_LOCAL_APPEARANCES=1\|menu\|reveal\|confirm\|remove\|cancel` | Applied-local-appearance warning, highlight toggle and confirmed removal |
| `AURORA_DEMO_REZ=<mode>` | Rez by drag and drop from the inventory |
| `AURORA_DEMO_LAND=<tab>[:role][:classic]` | "About Land" on a page or a tab, as owner, officer, member or visitor |
| `AURORA_DEMO_LAND_PRIMS=<used>[,<step>]` | Objects of the demo parcel, which increase with each answer |
| `AURORA_DEMO_REGION=overview\|manage\|people[:role][:preview][:compact][:section][:pending][:ask-…]` | "Region / Estate" as estate owner, manager or resident |
| `AURORA_DEMO_GROUP_PROFILE=<scenario>[:members\|:invite\|:notices\|:compose\|:admin\|:identity\|:land\|:experiences]` | Group profile window, as a member, officer, owner or guest |
| `AURORA_DEMO_GROUP_CREATE=form\|cost\|refused\|limit\|created[:classic]` | "Create a group" window and its simulated server |
| `AURORA_DEMO_SOCIAL_MENU=<step>` | A resident's "Invite to group", "Add to contact set", "Block particles" and calling card |
| `AURORA_DEMO_LAND_MODERATION=menu\|none\|freeze\|eject\|eject-ban` | "Freeze" and "Eject" of a resident on our land |
| `AURORA_DEMO_PLACE=<place>` | "Places" on a place profile |
| `AURORA_DEMO_PLACES=<tab>[:wide,open]` | "Places" on a tab, widened, its first place selected |
| `AURORA_DEMO_LANDMARK=<mode>` | "Create landmark" window, or the lit landmark star |
| `AURORA_DEMO_PLACE_WINDOW=1` | Place profiles in separate windows |
| `AURORA_DEMO_TP_CONFIRM=<origin>` | Teleport confirmation on screen |
| `AURORA_DEMO_SEARCH=<tab>[:text]` | "Search" on a tab (or Aurora's "all" chip), search done |
| `AURORA_DEMO_SEARCH_WEB=<page>` | Page loaded by the Web tab of "Search" in demo |
| `AURORA_DEMO_GRID_HELP=<page>` | Help › Knowledge base shown, opening this page |
| `AURORA_DEMO_MARKETPLACE=<page>` | Marketplace window open at start on this page |
| `AURORA_DEMO_MARKETPLACE_LISTINGS=<mode>` | Marketplace listings window open on a simulated SLM server |
| `AURORA_DEMO_BAN=1` | Ban lines |
| `AURORA_DEMO_RESTRICTED=1` | Parcel that forbids everything |
| `AURORA_DEMO_FREEZE=1` or `<seconds>` | The avatar frozen by an estate manager, then released |
| `AURORA_DEMO_SIM_CONSOLE=1` or `open` | The region console of estate managers (Advanced › Region console) |
| `AURORA_DEMO_GRID_STATUS=open\|new\|none\|error` | Help › Grid status: the grid's incidents, and its toolbar button's flash |

### Details

- **`AURORA_DEMO_RLVA`**: `enabled` opens Preferences > RLVa and
  applies synthetic object restrictions at frame 700. `disabled` delivers
  the same commands while the global opt-in stays off; `released` enables
  them, then switches RLVa off at frame 760. Capture after frame 800 in EN
  and FR: verify the switch, the active list and the empty released list.
  This scenario never saves the opt-in to the normal viewer's preferences.
  `im-query` applies fly/IM restrictions and delivers an online synthetic
  `@list` request at frame 740. The native permission card offers sharing
  once, remembering consent, or declining. Capture after frame 800 in EN/FR.
  `rename` turns shared wear off and enables automatic attachment naming.
  Two new shared objects arrive at frame 740. By frame 810 the log reports
  `Modifiable bracelet (chest)` and the no-modify object's `.(skull)` folder.
  `shared-offer` opens the ordinary confirmation for a synthetic scripted
  `#RLV/~Demo/Cuffs` folder gift. `shared-offer-accepted` accepts it at frame
  740 and delivers the matching inventory transaction; by frame 810 the log
  must report `path=~Demo/Cuffs`. The native offer and shared-gift preference
  use the same EN/FR controls as real offers. No grid is contacted.
  `experience` attaches a synthetic General-rated experience controller,
  applies fly/jump locks and identifies it through attachment resources at
  frame 740. By frame 810 the log reports `blocked=true, fly=false, jump=false`.
  `experience-allowed` uses a Moderate rating: `blocked=false, fly=true,
  jump=true`. `experience-released` also disables RLVa at frame 760, clearing
  the controller block and all restrictions. Capture after frame 810 to
  inspect the rating, temporary-controller and blocked-list controls in EN/FR.
  `chat` shows synthetic OOC, restricted ordinary/emote and directed-object
  messages in nearby chat. The log reports `ooc=true, direct=true,
  filtered_rows=2`; `chat-silent` disables ellipses and reports zero filtered
  rows while OOC and directed chat remain visible. `version` logs legacy/new
  handshakes with numeric `3040300` and implementation `2040213` at frame 740.
  `version-compat` seeds an owned attachment matching a creator compatibility
  entry and logs numeric `2092800`, retaining the same implementation number.
  `chat-home` enables the native `tph` shortcut, applies tplm and invokes the
  actual chat-command route. Its log must report `handled=true,
  requested=false, pending_ui=false`: no teleport or progress UI starts.
  `tools` opens the native RLVa console and exercises local version/status
  queries, duplicate feedback and a console-owned fly lock alongside another
  issuer's jump lock. At frame 810 its log reports two local replies and no
  channel-zero network reply. `tools-closed` closes the console at frame 810
  and reports `fly=false, other_jump=true`. Capture after that frame in EN/FR.
  `named-blocked` starts a named lookup, adds tploc and then resolves a distant
  destination; `maturity-blocked` holds a home teleport for a maturity response
  and adds tplm before releasing it. Both exercise the real deferred application
  routes and log `requested=false, pending_ui=false, failed=true` at frame 810.
  `communication` applies receive/send/start IM restrictions, receives an
  ordinary message and sends a reply through the native action boundary.
  Capture after frame 800: the conversation contains localized replacement
  text for both messages. It also attempts a new IM through the actual
  context-menu action before receiving the message; that must not create
  an exempt session. The log reports `new_session_blocked=true`, `local_placeholders=true` and
  `existing_session_allowed=true`. `temporary` disables temporary controllers
  and delivers OwnerSay from an attachment with the native temporary marker,
  without experience or inventory metadata. Its raw command remains nearby
  chat; the log reports `option_off_jump=false, source_allowed=false`.
  `hud-lock` seeds a temporary HUD with a point removal lock while requesting
  hidden HUDs and wireframe rendering. Capture after frame 810: the HUD is
  visible, the world stays filled and the log reports `locked=true,
  shown=true, wireframe=false`.
  `sphere-blend`, `sphere-blur`, `sphere-variable`, `sphere-chromatic` and
  `sphere-pixel` close Preferences at frame 700 and apply each GPU vision
  mode to the same scene. `overlay` uses the demo's transparent-hole texture
  with scripted opacity and touch blocking: world clicks through holes
  remain available, opaque texels block them, and HUD/UI clicks stay usable.
  Capture after frame 800; compare the same scene with `disabled` and close
  Preferences for the unobstructed baseline. These modes never contact a grid.
  Vision modes close Conversations and use the same fixed camera framing
  around the avatar so its body and attachments remain visible.
  `baseline` shows the unobstructed scene directly. `textures`, `silhouette`,
  `self` and `head` exercise forced world textures, distant avatar silhouettes
  and own-avatar visibility. `resolution` sets the real 3D resolution divisor
  to 12: the scene becomes coarse while text, HUDs and controls stay sharp.
  Compare it with `baseline` and `resolution-released`; check scene clicks
  still address their visible surfaces. Append `-released` to an effect mode to disable
  RLVa at frame 760 and verify restoration in the same run.
  `privacy` hides nearby identities and the current place in Conversations
  and the top bar; `privacy-except` permits one visible speaker through a
  strict name exception. `privacy-released` restores names and location.
  `nearby` opens People with an empty Nearby tab under shownearby. Inventory
  is closed in these scenarios so the conversation and list remain visible.
  `camera-focus` focuses a posed avatar's head at a scripted distance and
  direction with FOV limits. `camera-prefs` opens Camera preferences under
  an exclusive camera lock: preset, offset, distance and viewing-angle
  controls must be disabled while unrelated options remain editable.
  `sitting` sends a forced object sit at frame 700, then sitground at 740.
  The native demo simulator must confirm the stand before the ground sit.
  At frame 810 its log reports `ground_sit=true, object_seated=false`;
  capture after frame 850 to inspect the final ground-sitting pose.

- **`AURORA_DEMO_GRIDS=1|pages`** opens the grid manager with synthetic metadata
  without contacting a grid; `pages` selects its Web pages tab. Combine with
  `AURORA_DEMO_LOGIN=cards8` to inspect the dialog over the account selector.
  Inspect the narrow catalogue and Connection / Web pages tabs; add, edit
  and remove entries. Wheel, arrow keys and dragging must not browse accounts
  behind the dialog; normal browsing resumes after Close. Saved grids
  appear in both login layouts and account editors. Outside demo, catalogue
  editing requires logout. Human validation covers real login, the grid's
  search/welcome/help pages, same-grid hops and cross-grid Hypergrid travel.

- **`AURORA_DEMO_PERF`**: `compact` opens the compact view (frame rate and
  its graph); `full` the resource monitor on its overview; `network` the
  monitor's Network section as the bars open it (the full view for this
  opening only); `monitor-<section>` one section of the monitor:
  `overview`, `subsystems`, `threads`, `memory`, `network`, `disk`, `scene`
  or `region` (`scene-objects`: the scene on its objects). The monitor samples every half second: allow
  `AURORA_FPS_LIMIT=60` and `AURORA_CAPTURE_FRAMES=1500` (25 s) for graphs
  with some history, and `AURORA_WINDOW_SIZE=1600,1000` so that its default
  size (1180 × 760) fits. What to look for: the rail's eight sections with
  their live figures; the overview's six tiles and their minute; the
  subsystems' table sorted by a column (click a title), its parts unfolded,
  the selected row's pane (threads, related settings, processor graph);
  the threads' groups and the per-core bars; the region's simulator health
  (the demo's simulator sends its statistics when the monitor asks). The
  demo has no circuit: the network's UDP counters stay empty but for its
  synthetic traffic, and no media process runs.

- **`AURORA_DEMO_PAY=<mode>`**: gives a balance of L$ 1,000 and opens the
  Pay window, Tess as the payee (her profile picture on the card), in the
  look of Preferences › Aurora › Pay window. A payment only moves the demo
  balance (no L$, no request sent).
  `resident`: L$ 25 typed, Pay sends it and the window says it was sent.
  `empty`: no amount typed (the field's "0" hint, in the figures' size).
  `short`: L$ 1,500, the band with Buy L$ and Pay greyed out. `confirm`:
  L$ 500, the confirmation asked above the threshold (Preferences ›
  Payments). `sent`: the end of a payment, with its message. `group`: the
  Northern Wolves group as the payee (its insignia on the card), without a
  message field. An object's payment
  is `AURORA_DEMO_ACTIONS=pay`.
- **`AURORA_DEMO_MONEY_TRACKER=<mode>`**: opens Me › Money tracker on twelve
  made-up transactions of the last 75 minutes (tips through an object,
  gifts, a sale and a purchase, an upload fee, a group's joining fee).
  `aurora`: the Aurora look (totals, filter, newest first). `legacy`: the
  Legacy Viewer's Time / Name / Amount list. `select`: two rows selected,
  "Copy 2 (CSV)". `undo`: two rows removed and the bar that puts them back.
  `empty`: nothing recorded yet. A resident paid with the Pay window during
  the demo is added as a paid gift while the tracker records.
- **`AURORA_DEMO_PAYMENT=<mode>`**: a resident's direct payment as a card in
  their conversation (#1198), the Conversations window in sight. `card`:
  Tess's conversation open on her message, then two cards: "paid you
  L$1,500" with "Thanks for the set!", and L$75 with no message (the
  simulator's "Payment" is not shown). `legacy`: the same as two lines of
  the legacy lines, the coin before the sentence. `sent`: the same, then
  our own L$300 to her, "You paid … L$300" with "For the rent", not
  counted as unread (the demo keeps no transcript: that both payments come
  back with the history is a unit test's, and a grid check). `sent-open`:
  no conversation open; our L$300 to Tess opens hers on its card, with
  "Open the conversation when you pay a resident" turned on (off by
  default). `friend`: no
  conversation open; a friend's L$250 opens theirs on its card ("For the boat ride"), a
  stranger's L$20 shows nowhere. `off`: the same with "Open a conversation for a
  friend's payment" off: no conversation opens. Capture after frame 400, with `--language fr`
  and `--language en`.
- **`AURORA_DEMO_BUY_CURRENCY=<mode>`**: gives a balance of L$ 479 and opens
  "Buy L$". The purchase service is simulated (made-up rate): no real L$,
  no request sent. `click` opens nothing: click the balance in the top
  bar, or Me › Buy L$…. `empty`: empty amount, "approx. US$ 0.00", Buy
  greyed out. A number (`1000` by default): amount entered and estimated
  cost; when an amount is typed again, "Estimating…" stays for 2 s after
  the last keystroke. `confirm` and `password`: confirmation open, plain or
  with a password (OK greyed out as long as it is empty). `direct`: purchase
  without confirmation. `error`: refusal by the service shown in the
  window, with its link. `nopayment`: account without a payment method.
  `grid`: grid with no known purchase service. After Buy then OK: the
  window closes, the balance goes up and the bell receives the
  notification.
- **`AURORA_DEMO_BUY_LAND=<mode>[:confirm]`**: the plaza parcel (16384 m²) is for sale
  by a demo resident for L$ 4,500, with its objects, and "Buy Land" opens on it.
  The land service is simulated: nothing is sent, the purchase only moves
  the demo balance and gives the parcel to the avatar (About Land then shows
  it as yours). The demo estate has a covenant: Purchase stays greyed out
  until "I agree" is ticked. `rich`: L$ 12,000, enough to buy, a "click"
  confirmation. `short`: L$ 1,200; the third step proposes the L$ 3,300
  missing with their estimated cost, an amount typed again is estimated
  2 s after the last keystroke. `upgrade`: the account must change plan
  (list of plans) and the land use fee goes up, confirmation with a password
  (OK greyed out while it is empty). `other`: the parcel is set to sell to
  Nova only, "Not for sale" with the reason. `error`: the land service
  refuses, with its "Go to website" link. `:confirm` after a mode (`rich:confirm`,
  `upgrade:confirm`) ticks the covenant and opens the confirmation by itself,
  for captures.
- **`AURORA_DEMO_BUY_PASS=sell|none|own`**: at frame 240, the ground menu's
  "Buy access" where the avatar stands, on the plaza parcel, without About
  Land. `sell`: Loup's parcel sells passes of an hour for L$ 88; the
  question shows the price, the parcel's name and the hours, OK logs
  `demo action: ParcelBuyPass` (nothing is sent). `none`: Loup's parcel
  without passes, `own`: the avatar's; both show a notice saying why instead
  of the question. Capture at frame 620. `AURORA_DEMO_RCLICK="x,y"` on the
  ground shows the entry in the menu.
- **`AURORA_DEMO_IMPORTS=launcher|settings`** opens the empty unified import
  window or its Permissions tab. Ctrl+U and Build > Import follow the saved
  Preferences > Inventory > Open imports with choice (Upload window by default).
- **`AURORA_DEMO_IMPORTS=icon-bar`** opens the movable quick import bar without
  a disk dialog. The Aurora glass surface groups flat action buttons and
  a grip for moving the bar; the open inspector or Permissions is accented.
  Texture, sound, animation, material and mesh each open their own window,
  reusing existing drafts or requesting the corresponding file filter if empty.
  Local previews have a separate window; Folder opens the unified uploader
  and immediately asks for a directory. Each type window retains its own
  selection, tab and geometry, with fees and confirmation limited to that type.
  The uploader's Quick import bar switch changes
  the same saved entry-point preference and shows or hides the strip at once.
  The preference is at the bottom of Inventory, after Received items.
  Closing a window leaves the shared drafts intact.
- **`AURORA_DEMO_IMPORTS=type-texture|type-sound|type-animation|type-material|type-mesh|type-local`**
  loads that type's synthetic draft into its dedicated window with the bar visible.
  **`type-windows`** loads a mixed queue and places texture and sound windows
  side by side (1600×1000). Check independent selections, titles, tab switches,
  close/reopen, type-filtered Add files, and confirmation while another window is open.
- **`AURORA_DEMO_IMPORTS=unified`** loads a mixed queue with a mesh first,
  followed by a texture, sound, BVH animation and material. The first row is
  selected automatically. Click each queue row: the right inspector follows
  the selection in Files, below the destination options. The right pane has
  Files / Folder preview / Permissions tabs; switch among them without losing the active
  file or mesh section. Folder preview shows the resulting inventory hierarchy
  with optional path edits; Permissions contains the per-type rights and payment
  preference, without opening another window. Mesh preview,
  LOD, avatar and physics controls retain their established structure.
  Toggle the type icons: they change visibility, preserving upload checkboxes.
  Use default folders starts checked and remembers the last choice. Uncheck it,
  choose an inventory root and mirror/type/single-folder routing. These options
  are shared by Files and Folder preview; switching tabs must never change
  destinations. With single-folder routing, imported files and their mesh/material textures
  share the source wrapper when included, or the chosen destination when omitted. Recheck defaults to restore per-type targets;
  uncheck again to restore edited paths. Price and import actions stay pinned at
  the bottom of Files and Folder preview and are absent from Permissions.
  The larger type icons wrap in a narrow column. Each queue row aligns its
  checkbox, type icon and filename on one line, without a repeated type label or
  Ready subtitle; hover retains its path/status, and non-ready rows retain their
  progress/error indicator. The trash icon beside the queue count removes all files
  in the current upload/local mode, including hidden and unchecked rows; local
  previews stay when clearing uploads. It is disabled while loading or busy.
  Neither upload nor local queues expose sort/reverse controls.
  Check Add files, Add folder, insertion order through filter changes, removal and
  return to the mesh without losing its section/options. Local previews use
  the eye toggle in this same window; the Permissions tab retains the rights settings.
  In local mode, check several files (including a hidden type) and choose Add to
  upload (count). Only checked, ready local files are copied; the view switches to
  Files on the first upload draft and reveals its selected types. Original local
  previews stay applied and available through the eye toggle. Material edits and
  individual materials from the same glTF, mesh LOD/physics options, names and
  descriptions survive. A material using a local texture must upload its pixels as
  a dependency, even when that texture is unchecked; server texture references
  stay existing assets. Upload drafts have fresh IDs and independent mesh fee/job
  state; staging sends nothing and spends nothing. The ordinary destination,
  quote and paid confirmation flow follows. No selection, loading and busy states
  disable the action; exceeding queue/input budgets creates no partial batch.
  Empty the queue and complete an upload with its payment receipt visible:
  the window keeps its size, and the controls below the queue stay reachable.
- **`AURORA_DEMO_IMPORTS=loose-folder|loose-folder-preview|type-sound-folder`**
  shows files added without a disk folder, grouped under a named Release folder.
  Uncheck Use default folders, toggle Create folder and enter a name. The name
  updates the destination immediately, with no Apply button; clearing it or
  entering an invalid name blocks upload until corrected.
  There is no Include source folder control for loose files. Check Files and
  Folder preview, and the dedicated sound window: changing the name or disabling
  grouping retains manual inner paths and never changes disk paths. An enabled
  unnamed folder blocks uploads. Folder creation starts only after confirmation,
  and uploads wait for acknowledged inventory UUIDs. Selected disk directories
  keep their own Include source folder option, including in mixed queues.
- **`AURORA_DEMO_IMPORTS=sound-confirm|material-confirm`** opens the confirmation
  over a sound or a material that reuses one image in multiple maps. Only the
  material shows the shared-texture note. The note follows selected files and
  effective texture encoding, not merely the presence of a texture or material.
- **`AURORA_DEMO_IMPORTS=inspectors`** loads every type, including local
  previews, in the same window. Queue selection replaces the former type tabs.
- **`AURORA_DEMO_IMPORTS=texture|destination|material|material-cube|material-aurora|material-maps|material-loading|material-zoom|material-texture-zoom|mesh|mesh-normals|mesh-zoom|mesh-low|mesh-error|mesh-physics|mesh-degenerate|mesh-options|mesh-avatar|mesh-avatar-rigged|mesh-journal|permissions|animation|animation-own|sound|bulk|bulk-mesh-denied|bulk-upload|local|local-mesh|local-mesh-applied|local-mesh-removed`**:
  writes files specific to this launch into `.scratch/import-demo/<uuid>/`
  then opens the chosen flow: RGBA image, PBR material, Collada cube,
  BVH / `.anim` animations and synthetic WAV sound. Check the settings, the
  previews, the camera, the permissions, the selection and the confirmation.
  With `texture` and `AURORA_DEMO_IMPORT_FILE`, also check grayscale and
  grayscale/alpha PNGs, and 16-bit grayscale, RGB and RGBA exports: all must
  show a preview without a channel error. Luminance is repeated in RGB,
  alpha is retained, and 16-bit channels use their high byte, following
  `LLPngWrapper::normalizeImage`. Ordinary 8-bit RGB/RGBA files and material
  maps must still load with the same colours and transparency.
  The texture inspector shows "Limit size to 1024 × 1024" only when the
  unrestricted power-of-two width or height would exceed 1024. It is off by
  default (2048 maximum). Each explicit change is saved; newly added textures
  capture the latest choice when they join the list, while existing files
  retain their own limit. Viewing a smaller image hides the switch without
  replacing that remembered choice. With a 2048 × 1024 source opened
  through `AURORA_DEMO_IMPORT_FILE`, check that toggling it changes the
  upload size between 1024 × 512 and 2048 × 1024 and updates the fee. Only
  the final power-of-two dimensions are shown, as in `LLFloaterImagePreview`.
  Every scenario uses the same window and queue. Clicking a row in the left queue opens Files, including the already active
  item from Folder preview or Permissions; checkbox/filter changes retain the tab. Files shows its
  asset inspector; Folder preview shows that file's destination and can edit its path.
  The left column’s **Add folder** action scans supported
  files recursively on a worker. The selected source folder is retained in
  the editable inventory preview; unsupported files, empty directories and
  links/reparse points are skipped. Existing batch limits remain 256 source
  files per selection, 512 drafts and 1 GiB of retained non-mesh data; scans
  also stop explicitly beyond 100,000 entries or 64 directory levels.
  **`AURORA_DEMO_IMPORTS=folders|folders-lods|folders-type|folders-flat|folders-upload`** supplies a
  nested Creator Pack with equal Fabric names in Coat and Hat, a mesh, an
  animation and a sound. Recognized configured LOD/physics companions are
  attached to the mesh and omitted from the queue and preview; invalid or
  unmatched files stay visible. `folders-lods` selects the mesh in Files;
  `folders-type` starts grouped by asset type inside Creator Pack, retaining
  the imported folder's name as in mirror mode. Different imported root names keep
  their own type folders; individually added files group directly by type.
  `folders-flat` places all imported contents directly inside Creator Pack.
  Adding a folder keeps the currently selected tab. Under Destination, Include
  source folder defaults on and remembers its last checkbox state across restarts.
  Toggle it in Files and Folder preview for all three layouts; off removes only
  the source wrapper. Rename it beside the checkbox and apply: the inventory plan
  changes, the PC directory does not, and manual inner paths survive. For multiple
  source names, select which wrapper to rename; loose files have no wrapper.
  `folders-upload` exercises
  confirmation, folder acknowledgements, mesh requoting and the existing paid
  queue entirely offline. The other modes never create or upload
  automatically. Check mirror/type/single-destination arrangements, selection
  in the left queue, and selecting a folder then editing its whole
  relative path. Folder preview has no sorting or All/None controls and no
  per-file checkboxes. Each file row shows its destination beside its name,
  with a relative-path field and Apply action; defaults show real inventory
  paths while editing is locked. Test equal names with independent unfinished
  edits, invalid paths, an empty path targeting the chosen root, and busy-state
  locking. Applying one row must not select it for upload or change another row.
  The former active-file editor below the list is removed. Empty relative
  paths target the common destination directly. The preview includes actual
  inventory ancestors above proposed children; per-type default mode shows
  each real target. Calculate mesh fees, then confirm upload: missing folders
  are created automatically using ordinary inventory-name normalization, parent
  first. No folder is created during inspection or fee calculation. After the
  acknowledgements, meshes refresh their quotes against actual destinations;
  a higher price stops before paid transfer and requires a new confirmation.
  Cancellation keeps acknowledged folders and stops subsequent creations;
  failures do not start uploads. On the grid (human only), verify nested inventory
  destinations, material/mesh texture destinations, protected folders and
  failure/cancel/retry before confirming the paid operation.
  `material-cube` / `material-aurora` select the additional preview shapes.
  `material-maps` shows its four map editors beside the preview in a 2 × 2 grid;
  a narrow detail pane uses unframed rows below a compact preview, with
  95.2-point thumbnails preserving original proportions. Check the left-aligned
  titles and texture names, right-aligned commands and common parameter rows.
  Metallic / Roughness use only numeric fields; Normal appears only in
  its row title. The narrower vertical inspector uses a 176 × 176 canvas;
  `material-loading` keeps that same canvas
  and gesture tooltip while the model is pending. Multiple keeps its wider grid.
  The preview model selector offers Sphere, Cube and Aurora 3D. Drag to
  orbit, Shift + drag or the middle button to pan in screen coordinates,
  wheel to zoom; Reset view and double-click reset the full camera.
  `material-zoom`, `mesh-zoom` and `material-texture-zoom` open the loupes.
  Texture pixels are prepared on a worker at native resolution, capped
  to 2,048 per side, without upscaling; closing or switching frees the cache.
  `permissions` is an alias for Settings: Modify / Copy / Transfer for
  Texture, Sound, Animation, Mesh and PBR Material, plus group sharing and
  everyone-copy columns within each type's row. Each row is saved independently; disabling
  Copy requires Transfer. There are no rights controls in any inspector.
  Settings contains only the permissions table and payment preference.
  `mesh-colors` opens the mesh palette floater. Click each pastille, then edit
  through the viewer's common picker: the palette remains open and the preview
  updates immediately. Background, edges, physics fill/edges and degenerate
  fill/edges follow their selected colors without rebuilding the model.
  `sound` loads a six-second decaying tone: inspect the waveform, drag crop
  markers, change numeric bounds, adjust import volume up to 400% and reset. The playhead
  follows the prepared clip; cropping stops it until Play is clicked again.
  Gain commits restart playback with the processed, ceiling-limited PCM.
  `sound-split` selects 0.25–5.5 seconds, boosts to 200% and splits at two
  seconds: verify orange peak indication, division markers, three named assets
  (`_01`–`_03`), the short tail and a three-asset price. Split length is limited
  to the grid's asset maximum; long WAV sources enable splitting on load.
  `animation` shows aligned settings beside its preview, with loop units
  labelled and easing fields aligned. A mixed queue containing meshes needs their
  quotes before confirmation. Ctrl+U toggles the saved entry point (the unified
  window by default, or the quick import bar). Add files requests a file picker;
  an empty type icon requests its filtered picker and Folder requests a directory.
  Rebind the shortcut in Preferences › Shortcuts.
  The quote and the uploads stay entirely inside the demo server.
  In `destination`, uncheck Use default folders to open the inventory destination
  tree, with the current path expanded. Toggle a parent caret: only its children fold/unfold; the selected
  destination and popup stay unchanged. Search for a nested folder: its ancestors
  remain visible, and duplicate names keep their own paths/UUIDs. Typing must
  not change the destination or close the popup. Choosing a name applies that
  folder and closes the popup. Trash, protected/library/outfit/inbox subtrees
  remain unavailable. Check that inventory renames, protection changes and sort
  preferences refresh the tree. In Folder preview, folding the result must
  preserve upload selection and relative paths.
  All types retain one window geometry and share the visible queue.
  Destination options stay above the content in Files and Folder preview.
  Mesh metadata and tabs keep their existing layout. Preview and LOD cards are side
  by side, with gesture hints directly under the preview and compact tools.
  High is shown by default; clicking anywhere within a LOD card selects
  its preview, including its fields and counters. File / Browse share
  a row, generated criteria / targets are below; results and retained
  percentage are right-aligned, and triangles are spelled out.
  `mesh-normals` shows 105° without a stray character or clipping.
  In `mesh-physics`, open the collision-source list: File is first, followed by
  the other existing sources. The current choice and the initial convex-only
  default stay unchanged; File exposes Browse for a local physics mesh.
  `bulk-mesh-denied` simulates a connected account without mesh upload
  rights, with a mesh and a texture selected. The footer reports the
  mesh refusal; deselecting the mesh allows the texture upload.
  Recalculating fees simulates the same refusal.
  `mesh-low` shows Lowest to really compare the geometries and
  their counters. Choosing "File" for a generated LOD must open the
  file picker; cancelling keeps the LOD, choosing a valid file restarts
  the preview even after a previous error. The option to preview on our
  avatar stays the same when changing animation and after a restart.
  Its checkbox is to the right of the Play / Stop / Pause controls. The tabs
  use the same theme as the inventory. Changing the number of triangles
  keeps the model during the recomputation and does not move the controls;
  the error threshold carries a percentage unit and a hover help.
  `mesh-error` shows an HTTP 400 refusal with a validation code and an
  explanation of the physics shape, without any network request.
  `mesh-options` opens and frames the third section (size, textures, skeleton,
  file matching): two columns of settings, aligned suffixes
  and help texts at normal size. Check that all the fields can be read
  without their text being cut, and their layout after shrinking the window.
  In the list of animations, clicking a file
  starts its playback, even if that animation had already been played.
  `bulk-upload` runs the batch automatically on this offline server:
  five successful files, material dependencies and an expected receipt
  of 51 fictitious L$. Uploaded files leave the list after acknowledgement;
  reopening stays empty. Local previews and unselected / failed /
  cancelled drafts remain; no real transfer or payment.
  `mesh-physics` shows the physics shape and its edges; `mesh-degenerate`
  injects a triangle defect, which must appear in the preview and block
  the quote with a diagnostic. The convex analysis must allow it to be solved.
  The "Reload file" button re-reads these files through the normal path. For
  local watching, enable the preview then modify its file or a material
  map; the map's local identifier must stay stable.
  `local-mesh-applied`, with `AURORA_DEMO_ACTIONS=1`, automatically applies
  the local cube to the selected test box; its server volume stays
  unchanged. The remove button must restore its original appearance.
  `local-mesh-removed` follows the same flow then removes the preview at frame
  400. The aggregated traces check the GPU binding to the local mesh, then the
  return to the prim, with the server volume kept; no file name or
  personal path is added to them.
  The modes `local-texture-applied|local-texture-removed|local-texture-rez`
  and `local-material-applied|local-material-removed|local-material-rez`
  follow the same demo object: active GPU texture/material, then restoration
  or detection of the supporting prim. They require `AURORA_DEMO_ACTIONS=1`.
  The traces also check that the server data received no local UUID.
  Add `AURORA_DEMO_LOCAL_APPEARANCES=1|menu|reveal|confirm|remove|cancel`
  to `local-material-applied` (or `local-texture-applied`) to hide the import,
  People and Inventory windows and focus the preview object at frame 600,
  then exercise the warning through real pointer events.
  Capture frame 670 for the menu, 700 for the highlight/confirmation and 740
  for removal/cancellation. The warning has a direct highlight toggle;
  `reveal` switches it on at frame 680 and off at frame 850 without opening
  the menu. Capture frames 700/800 while pulsing and 950 after switching off.
  Highlight stays enabled until switched off and respects Reduce motion.
  `local` loads texture, material and mesh together through the same filter.
  `AURORA_DEMO_IMPORT_REGISTRY=<test file>` enables the persistence of the
  import scenarios to this isolated file. First launch `local-texture-applied`,
  `local-material-applied` or `local-mesh-applied`, then `local-restored` in
  a new process with the same file and `AURORA_DEMO_ACTIONS=1`.
  The second launch creates no new asset: only the library and
  the saved links must restore the GPU application. Without this
  variable, the synthetic scenarios do not fill the persistent library
  of the demo profile. The synthetic sources stay in `.scratch`.
  Texture/material applications expire seven days after their last application,
  independently per object/face. Reconnects, autosaves and source reloads do not
  extend this deadline. Older registries receive seven days on first migration.
  For an isolated expiry check, close the first process, set its saved
  `appearance_expirations` deadlines to a past Unix timestamp, then run
  `local-restored`: the source library stays, the warning disappears and the
  original server appearance returns. A future deadline must restore normally;
  a deadline reached while running must remove the application too. Never edit
  the normal account's registry for this test. Local mesh bindings are unchanged.
  `animation-own` puts the preview at 0.75 s, paused, also on the demo
  avatar. The slider and the pose must stay fixed until Resume.
  `animation-custom` loads a local rigged Bento tail and its looping animation,
  with the saved-model list open to show the removal icons at each row's right.
  Its tail must move in the preview. Choose model loads a rigged DAE, glTF or
  GLB from disk; its initial camera includes the bones' rotational reach, so a
  tail or wing must remain in view while turning. Standard restores the
  ordinary mannequin. Check seeking,
  pause/resume, base animations and camera controls with both models. Cancel
  the file picker or choose an unrigged/invalid file: keep the current model,
  with an error for rejected files. Model selection applies to every queued
  animation, including later additions, and remains preview-only: it never
  adds a mesh upload or changes its fee.
  Add two models, switch between them in the model list, then return to Standard.
  Restart the viewer and import animations: the last successfully used model
  loads automatically, without reopening a file picker. Standard applies to
  the whole queue and remains the default after a restart.
  Remove an unselected entry with its row's icon: the current preview must stay
  selected and the list must stay open. Remove the selected entry: return to
  Standard. In both cases its source file must remain on disk. Long names must
  leave room for the aligned removal icons. A moved/missing source
  stays in the list, reports a load error and keeps the previous preview.
  `AURORA_DEMO_IMPORT_FILE` replaces the fixtures with an existing file,
  read without modification; choose the mode matching its type.
  In `folders` modes it can point to an existing directory instead; its GLB/glTF
  files produce separate mesh and PBR material drafts. Toggle Mesh and Material
  visibility independently, and use row checkboxes to select what to upload.
  Add files in the mixed uploader captures its Material/Mesh filters: material
  only extracts materials, mesh only loads geometry, both extracts both, neither
  skips GLB/glTF files. Dedicated type windows always use their own type.


### Import inspector layout

Mesh importer parity regressions use the `mesh_import_parity` test filter
with `cargo test --workspace mesh_import_parity`. They cover a DAE larger than
64 MiB, a GLB with a 129 MiB embedded buffer, dense/redundant geometry,
more than 256 instances, deep hierarchies, flat-axis normalization and actual
preview dimensions, generated-model settings, authored
LOD material subsets/order/suffixes and node identities, unused texture dependencies, crease and
area-weighted normals, large source skin palettes, DAE axis-independent bind
frames, and glTF shoulder orientation / inherited rest transforms.
Synthetic weight regressions cover High vertices losing their only supported
influence, nearest-weight lookup across source faces in normalized space, and
the resulting serialized weights. No external model data is used in fixtures.
The large-input test briefly retains several hundred MiB; its two fixtures
are generated and released sequentially. Use existing `AURORA_DEMO_IMPORT_FILE`
with `mesh` or `mesh-avatar` for native captures of a supplied rigged asset;
`mesh-avatar-rigged` deliberately replaces its skin with the synthetic fixture.
The Mesh Options tab must show generated models = 768 and transfer timeout = 600.
Rigged previews play the standard standing animation independently of the
wearer and other imports, in both the inspector and enlarged canvas. Offline
`mesh*` demos use the existing synthetic idle under that standard motion ID.
Enabling imported joint positions must apply offsets without adding spheres.
Only "Show joint positions" enables markers; they follow the animation, with
1 mm dots and a 16 mm selected joint. Physics previews show no joint markers.
The synthetic preview regressions also cover independent animation clocks,
the pelvis offset and bend diagnostic, and upload/preview flag independence.
Human grid validation: compare the same DAE/GLB in the Legacy Viewer, including
weights, optional joint positions/scale lock, authored lower LODs and a slow
large-file upload. Only the human connects and confirms a paid transfer.

`AURORA_DEMO_IMPORTS=mesh-avatar|mesh-avatar-rigged|mesh-journal` opens the
complete Avatar or Journal tab of the native import window. The rigged mode
uses a synthetic weighted cube and the ordinary geometry/preview worker.
`mesh`, `mesh-options` and `mesh-physics` exercise the compact LOD rows,
Options and the two-step collision workflow. Check the metadata/preview
header, vertical tools/settings popup, five tabs and fixed model/fee summary.
All mesh-inspector text must use at least the normal checkbox-label size,
including gesture help, LOD results/percentages, collision captions, Avatar
counts and quote metrics. Check that LOD rows remain on one line in the wider
mesh window and collision captions wrap without reducing their font size. Also
open the mesh inspector in a short viewer window at 100% and fractional Windows
DPI, then leave it untouched on each tab, including Collisions before/after a
fee quote. The frame, preview and summary must settle without manually resizing;
scrolling the tab content must leave the header and summary fixed, with no
second outer details scroll bar.
Avatar exposes dependent weights/positions/scale-lock controls, -3..3 m Z
offset, selectable joints and read-only per-model positions/conflicts.
Journal contains actual load/preparation/validation messages. Detailed mode
affects subsequent diagnostics, never filters existing history; calculation
starts a fresh log and warnings mark its tab. No grid connection is needed
for these automated captures.
`AURORA_DEMO_IMPORTS=mesh-multiple` loads three separated objects from one
Collada file through the ordinary loader. All three must appear in the preview
and in the instance count; fee calculation and the final transfer retain the
same instance list. With `AURORA_DEMO_IMPORT_FILE`, also check a rigged Collada
whose skin controller has no scene instance: the model must load rather than
show a file error. On the grid, upload a single mesh and a multi-object file;
the completed object must appear in inventory with no false verification
failure. Grid validation and any paid upload are performed by the human.

- **Local import fixtures**: the ignored test
  `local_asset_directory_loads_and_prepares_supported_files` reads the folder
  named by `AURORA_IMPORT_TEST_DIR`, as well as its `meshes` and `textures`
  subfolders. The `lowest/low/medium/high` LODs and the `physics` physics
  are matched by suffix; the source files stay intact.
  The result is aggregated, without personal names or paths. Native
  animations keep their bytes and their poses are sampled.
  Run `cargo test --workspace local_asset_directory -- --ignored --nocapture`.

- **Walkthrough of the window redesign**: open the inventory and select
  a folder in the tree, then an item; check search, permissions,
  context menu and the actions at the bottom. Enlarge and shrink the window:
  the tree stays single and takes the full width. In Appearance, switch
  between gallery / list / worn, select an outfit and try Wear / Add offline.
  In "About Land", go through the sections while keeping the summary
  visible. Then open Conversations, People, Places, a profile,
  Preferences and Environment; move, resize, collapse and restore
  the windows. The build window keeps its width across its six sections,
  including Shape for the advanced settings; long lists scroll in the body,
  the actions stay visible. Modal confirmations keep the blurred background.
- **`AURORA_DEMO_INVENTORY=link-icons|aurora-link-icons`**: adds original/link pairs
  for demo item types and all seventeen wearable subtypes, plus a folder link and
  a broken link. Expand the context-test folder and scroll through it: valid links
  keep their target's type icon and tint with the shortcut arrow at the lower left;
  the broken link keeps the generic link icon. Compare the Legacy and Aurora layouts.

- **`AURORA_DEMO_INVENTORY=hover-photo|hover-photo-object|aurora-hover-photo|aurora-hover-photo-object`**:
  hover the synthetic texture or an object with a saved picture after frame 600.
  Capture after frame 700. Both layouts must show the image without selecting
  or opening the item. Preferences > Inventory > Show pictures on hover is on
  by default; disabling it retains the text tooltip and stops picture requests.
  Objects without a saved thumbnail retain their text tooltip. The viewer does
  not rez objects or inspect their contained assets to manufacture a preview.
  On the grid, also check a snapshot, a linked texture, an object/folder with a
  thumbnail, an outfit with exactly one direct texture, and RLVa `viewtexture`.
  Hover images use a separate 32-entry cache, limited to 256-pixel RGBA copies
  (8 MiB), with least-recently-used eviction and 30-second idle expiry. Texture
  source references end once the copy is complete or the pointer leaves;
  unreferenced source textures follow the streamer's normal delayed eviction.
  Regression filter `inventory_hover_cache` exercises 1,000 successive images,
  real egui handle release, active/recent retention, expiry, shared world/UI
  source references, bounded workers and replies arriving after cancellation.
- **`AURORA_DEMO_INVENTORY=default-permissions|aurora-default-permissions`**:
  opens the shared Default permissions window over the Legacy or Aurora
  inventory. The final New-menu entry opens the same window. Check creation
  rows, no-copy/transfer rules and fixed environment-copy controls. Import rows
  edit the same saved values as the unchanged Permissions tab in import windows.
  New demo inventory items use their category's selected permissions.
- **`AURORA_DEMO_INVENTORY=bulk-multi|bulk-folder|bulk-permissions`**: selected items
  and folder context menus expose Bulk permissions; the last mode opens the
  shared type/permission controls directly. Apply/OK use synthetic inventory
  mutations offline. Folders include real subfolders; links are excluded.
- **`AURORA_DEMO_INVENTORY=search-selection|search-selection-keyboard`**:
  search for item 1490 in a 1,500-item folder, select its real row at frames
  650–651, then clear the query using the field's cross at 720–721 or
  Ctrl+A / Backspace at 730–731. Capture at 680 and 820: the selection stays
  highlighted, its parent folder opens and the full inventory scrolls to
  it. At frame 800 the log reports the empty query, selection and visibility.
- **`AURORA_DEMO_INVENTORY=properties-no-mod|properties-no-creator`**: the
  properties of a no-modify object made by another resident. `no-mod` shows
  its creator as a link; `no-creator` gives it no creator, as the simulator
  sends an object whose parts have several creators: the row reads
  "(unknown / multiple)", without a link, with the reason on hover.
- **`AURORA_DEMO_WINDOWS=1|restore|drag`**, with `AURORA_DEMO_INVENTORY=1`:
  double-click on the real title bar of the inventory at frames 660–665; `1`
  keeps the bar collapsed, `restore` expands it at frames 1000–1005,
  `drag` also moves it by 100 × 60 points at frames 780–782.
  Take screenshots at `620,740,900,1120` to check content, collapse, move and
  restored dimensions. The "−" button stays usable; the other
  title bar buttons and the content do not trigger the title bar double-click.
  `reach-top`, `reach-bottom`, `reach-right`, `reach-left` and `reach-down`
  instead drag the inventory by its title bar at frames 700–709 (no
  collapse) into the widest gap between the top islands, between the bottom
  ones, or far past the right, left or bottom edge, and release it there:
  with the islands the windows move over the whole screen, a title left
  under an island comes out below the top row (above the bottom one), and
  by default every edge stops the window whole on the screen. Capture at
  `690,760` with `AURORA_FPS_LIMIT=60`.
- **`AURORA_DEMO_WINDOW_EDGES=<l|r|b…>`**: the left (`l`), right (`r`) or
  bottom (`b`) edge lets the windows cross it for the run (Preferences ›
  Interface, "Windows stop at the screen edges", not saved): past a side
  `TITLE_GRIP` (96 points) of the title bar stays on screen, past the bottom
  the whole title bar. With `AURORA_DEMO_WINDOWS=reach-right` (`r`),
  `reach-left` (`l`) or `reach-down` (`b`).
- **`AURORA_DEMO_MINIMIZED=1|expand`**: double-clicks the real title bar of
  Conversations at frames 600–605; the window becomes a bar 200 points wide
  at most, outlined in the accent while messages are unread (the demo group
  message), and the toolbar button keeps its dot. `expand` then clicks the
  real Conversations toolbar button at frames 900–901: the window expands
  instead of closing (issue #269). Capture with `AURORA_FPS_LIMIT=60` at
  `560,760,1000`.
- **`AURORA_DEMO_SNAP=1|group|size|free|detach`**: drags the real Conversations
  window by its title bar to next to People (both must be
  open: first-launch layout, or `AURORA_DEMO_UI`). Frame
  700: press on the title bar with Alt held; until frame 900 the window is held 6 px
  from the right edge of People and 5 px below the line of its top: the
  violet guide (the two edges that are going to touch, the line of the tops,
  the outline of the place where it is going to land) appear, the window
  stays under the pointer; frame 900: released with Alt still held,
  it snaps against People, with no gap, tops aligned. Alt is released at
  frame 901. `group` then moves
  People by 70 × 50 between frames 1000 and 1100: Conversations follows, the
  hint says "Alt: detach this window". `size` instead drags the
  bottom edge of Conversations to 4 px from the bottom line of People: the
  edge snaps to it under the pointer, with its guide line. `free` omits Alt
  throughout: Conversations stays where dropped with the "Alt: snap and
  attach" hint during its drag; moving People afterwards leaves Conversations
  in place. `detach` attaches first, then moves People with Alt held:
  Conversations stays behind and the link is removed. Take screenshots at
  `800,960,1060,1160` with `AURORA_FPS_LIMIT=60`.
- **`AURORA_DEMO_RESIZE=corner|edge|drag|sweep|sweep-h|sweep-fast|sweep-right|sweep-bottom[,<window>]`**: from frame 600,
  the pointer rests 6 points inside the bottom-right corner of a real
  window (`corner`), 1 point inside its right edge, halfway down
  (`edge`), or presses that corner and holds it 40 × 30 further out until
  frame 1000 (`drag`). The window is Conversations by default, or the id
  of another open floater (`about_land_aurora` with `AURORA_DEMO_LAND=manage`)
  or `minimap`. At rest the corner carries a short arc of the frame in
  `muted_dim`; hovered or dragged, the arc grows and turns `violet_light`
  and the outline of the edges that move lights up in `violet_light`
  (the bottom and right ones for the corner, the right one alone for the
  edge), with the system resize cursor. Capture with
  `AURORA_FPS_LIMIT=60` at frame 780. `sweep` presses the corner and drags
  it 500 points in by frame 700 (down to the window's minimum), then 900
  points out by frame 900, a few points a frame; `sweep-h` does the same
  on the height (400 in, 600 out); `sweep-fast` drags it 520 points in
  and back out in 24 frames (about 45 points a frame), five times, then
  rests at its start; released at frame 1000. `sweep-right` and
  `sweep-bottom` do what `sweep` and `sweep-h` do, by the middle of the
  right or the bottom edge instead of the corner. On the video player
  window (`link_preview_player`, with `AURORA_DEMO_CONV_PREVIEWS=videos`
  and `AURORA_DEMO_PREVIEW_PLAY=window`) the picture keeps the video's
  shape at every captured frame of the drag (#941): the other dimension
  follows the edge, and the window stops at its minimum or at the screen
  on the video's shape, never stretched. With
  `AURORA_DEMO_CONV=aurora:group` and `AURORA_CAPTURE_FRAMES` listing
  frames (every 10 from 600 to 900, and runs of consecutive ones), what to
  look for in the Conversations window: nothing jumps at a frame; around
  600 points the column eases to the rail and back (back only past 640),
  the members' panel eases in or out with its room (out only 40 points
  under it), the head's buttons and the field follow the window's edge on
  the same frame.
- **`AURORA_DEMO_NOTIF=1`**: the "Aurora lantern" offer is from a
  resident: the object is already in the inventory (Objects folder) when
  the offer arrives. "Keep", or closing the offer in the list, leaves it
  there; "Reject" sends it to the Trash. With `AURORA_DEMO_INVENTORY=offer`,
  the inventory opens on the received object and the offer is still to be
  answered; `offer-reject` rejects it at frame 700, then shows the object in
  the Trash. Take the screenshot after frame 900. With
  `AURORA_DEMO_INVENTORY=offer-card`, the offer's card shows in the giver's
  conversation, opened in sight (Orion Example in the demo's names, Violet
  Wolf in the offer's entry): "gives you « Aurora lantern »", "Object",
  Keep / Reject, and no toast. `offer-card-keep` keeps it from the card at
  frame 700: the card greys with "Kept" and the entry leaves the list.
  The kept card offers "Show in inventory"; clicking it opens Inventory on
  the received object, clearing filters and expanding its parent path.
  A confirmation with the linked item name is also saved in the giver's
  transcript so it remains available after reopening the conversation.
  `offer-card-legacy-keep` exercises the same links in the Legacy lines.
  Pending, rejected and unidentified offers show no inventory shortcut.
  `offer-card-legacy` shows the card as one line of the legacy lines;
  `offer-card-legacy-reject` rejects it from its notification at frame 700:
  the line ends with "· Rejected". Capture at frame 900, with
  `--language fr` and `--language en`.
- **`AURORA_DEMO_INVENTORY=offer-show|offer-card-show|offer-object-show`**:
  "Show accepted items in the inventory" turned on for the run, the
  Inventory window closed. At frame 700 the offer is kept: `offer-show`
  from its entry under the bell (the "Aurora lantern" of a resident),
  `offer-card-show` from its card in the giver's conversation,
  `offer-object-show` from the entry of a "Gift dispenser" object's offer,
  whose gift the demo then adds to Objects. Inventory opens on the kept
  item, selected, its folder expanded and the filters cleared. Capture at
  frame 900.
- **`AURORA_DEMO_INVENTORY=offer-auto|offer-object-auto|offer-auto-show|offer-object-auto-show`**:
  turn on automatic acceptance for this run. The resident's lantern or the
  scripted dispenser's object is accepted on arrival, with no pending card,
  conversation or acceptance buttons. The two receipt-feedback settings are
  on by default: a private nearby-chat system line, an informational inventory
  notification and one InventoryOffer sound. Disable either independently;
  disabling both makes acceptance silent. The `-show` variants also turn on "Show
  accepted items in the inventory" and open Inventory on the delivered item.
  Capture at frame 620 for the notification, 900 for the delivered selection;
  with `RUST_LOG=info`, the log reports zero pending
  offers and cards.
  `AURORA_DEMO_OPTIONS=inventory:auto-accept` reveals the preference and opens
  its sub-options. Without an auto-accept scenario or a saved choice it remains
  off, with both sub-options checked and disabled. Test all four combinations
  of `auto_accept_inventory_chat` / `auto_accept_inventory_notify` in the
  isolated profile; closing a receipt must never send another acceptance.
- **`AURORA_DEMO_NOTIFS=object|object-own|object-group|object-history|legacy-object`**: one object inventory offer at frame 700, with the source name, owner and item. `object` shows Accept, Discard and Mute Owner; `object-own` omits owner blocking for our own object; `object-group` uses a group owner; `object-history` refuses the gift and opens History at frame 850, retaining the clickable owner; `legacy-object` exercises the same gift in Legacy toasts. Capture at frame 900 in English and French. Identity names are underlined links to their native profiles; clicking a name keeps the offer pending.
- **`AURORA_DEMO_NOTIFS=group-notice|legacy-group-notice|group-notice-saved`**: two group notices at frame 700. Aurora Builders (no insignia: the group glyph) from Tess Touch without an attachment, then Northern Wolves (its insignia beside the title) from Orion with a "Texture kit" object attached. Each card is titled "group : subject", its text starts with "Sent by" and the sender, both names are links; the attachment row shows its icon, name and kind with "Save to inventory" beside it, or under it when a long translation leaves the name no room. `legacy-group-notice` shows them in the Legacy toasts; `group-notice-saved` saves the attachment at frame 850: the card stays out with a greyed "Saved". Capture at frame 760 (900 for `group-notice-saved`) in English and French.
- **`AURORA_DEMO_NOTIFS=1|dnd|list|history|legacy|legacy-history`**: the notification
  center's mockup, at 60 frames a second. An IM from Nova at frame 700
  unrolls from the bell at the right end of the top bar (violet bar of the
  time left, 6 s for information, 12 s for what waits for an answer) and
  leaves around frame 1180 (a message toasts only by default: no count;
  with Preferences › Chat › Toasts › "Keep messages under the bell" it
  folds into the bell and the red count bounces); Orion comes
  online at 1190 (small toast at the top right by default, once the card
  has folded in; never in the center), a
  teleport offer at 1270 and an inventory offer at 1325 stack under the
  bell; a burst of five arrives from 2110, 0.45 s apart: three cards and
  "+ 2 more — see all". `dnd` turns do not disturb on at frame 640: grey
  bell with its moon, nothing unrolls nor rings, everything still lands in
  "To do", and the list opened at 2700 shows the banner. `list` opens the
  list at 2700; `history` answers four entries at 2650 (the teleport offer
  declined, the inventory offer kept, information read), then shows the
  History tab (day headings, states, "Show the place", "Show in
  inventory", "Put back in To do"). `legacy` plays the same with
  Firestorm's toasts (Preferences › Aurora › Notifications), for this
  session only; `legacy-history` opens its list on the History tab like
  `history`. A script menu (`AURORA_DEMO_DIALOG=12`) stays rolled out
  until answered. Hover the cards to see them pause. The history is kept in
  memory in demo; on the grid it is saved per account.
- **`AURORA_DEMO_OBJECT_IM=off|region|parcel|strict`**: the filter of
  Preferences › Privacy › "Block object messages from elsewhere", set for
  this run only (the saved setting is not changed): `off`, on for the
  region, on for the parcel, or on for the parcel with friends' objects
  blocked too. Seven object messages reach nearby chat at 60 frames a
  second, from frame 700 to 880, each saying when it should be hidden:
  another region's stranger, our own object and a friend's object in
  another region, a stranger in this region outside the demo parcel (the
  128 m square around the plaza) and on it, an llRegionSayTo from outside
  the parcel, and a message with no location. `off` shows all seven,
  `region` hides the first, `parcel` the first, fourth and sixth,
  `strict` the friend's too. Each hidden message logs one line
  `object message from … dropped`. Capture around frame 1000.
- **`AURORA_DEMO_PRESENCE=<place>`**: the friends' online / offline toasts
  of Aurora's notification center at `top-left`, `top-center`,
  `top-right`, `bottom-left`, `bottom-center` or `bottom-right`, for this
  run only (Preferences › Chat › Toasts is not changed), at 60 frames a
  second, with People and Conversations closed. Three nearby chat lines at
  frame 640, then three friends at 700, 730 and 760: the toasts start
  from their edge of the room the bars leave (below the top islands or
  bar, above the dock or the bottom bar) and stack away from it, never
  over a bar; at the bottom left they stack above the chat lines. An IM
  card unrolls under the bell at 1000 and folds in around 1380; at the
  top right the friend of 1060 waits, then is dropped (3.5 s without the
  place), the one of 1200 waits and shows once the card has folded. The
  bell's list opens at 1500 and closes at 1600: the friend of 1540 shows
  at the close; it opens again at 1700 until 1990: the friend of 1720 is
  dropped. The other places show every toast at once. Combine with
  `AURORA_DEMO_BARS=islands` or `legacy` for either look and
  `AURORA_WINDOW_SIZE`; capture with `AURORA_FPS_LIMIT=60` at
  `790,1100,1390,1560,1610,2000`. A very low window (`1280,360`) shows only
  the newest toasts that fit.
- **`AURORA_DEMO_FRIEND_PROMPT=1|later`**: the friendship offer in the
  middle of the screen (#465), turned on for this run only (Preferences ›
  Chat › Toasts is not changed). Nova offers friendship with a message at
  frame 700, Tess without one at 760: Nova's card arrives (it rises, the
  picture pops in, rings pulse, hearts rise for four seconds) and says one
  more offer is waiting; neither offer gets a toast, both wait under the
  bell. With `later`, Nova's offer is put off at 900 and Tess's card
  arrives. Capture with `AURORA_FPS_LIMIT=60` at 800 (and 960 with
  `later`), in `--language en` and `fr`; Preferences › Interface › "Reduce
  animations" leaves the card still.
- **`AURORA_DEMO_REGION_MESSAGE=1`**: messages to the whole region in the
  middle of the screen (#1404), turned on for this run only (Preferences ›
  Chat › Toasts is not changed). Orion, the estate manager, announces a
  restart at frame 700, Nova a party at 760: Orion's card arrives (the
  region's badge, his name, "sent a message to Aurora Demo", the message)
  and says one more message is waiting; neither gets a toast, both wait
  under the bell and are written in nearby chat. OK closes Orion's and
  Nova's arrives. Capture with `AURORA_FPS_LIMIT=60` at 800, in
  `--language en` and `fr`.
- **`AURORA_DEMO_SIM_VERSION=1|on`**: the region tells its simulator
  version at frame 640 (the first one, never told), then another one at
  700, as after a teleport to a region on another release. `1` follows
  Preferences › Confirmations › Simulator version change, off by default:
  no notice comes. `on` turns it on for this run only (the setting is not
  saved): the "Simulator version changed" card arrives at 700 with both
  versions. Capture with `AURORA_FPS_LIMIT=60` at 760.
- **`AURORA_DEMO_ABOUT=info|licences[:end]`**: the About window
  (`ui/about.rs`) opens at frame 600 on the tab asked for (`info` when the
  value is anything else); `:end` scrolls the tab to its last card. Info shows the demo region and simulator, the
  computer once `system_info.rs` has read it in the background (a
  "Reading…" line for a moment), and "Copy to clipboard"; Licences lists
  NOTICE.md, a card per heading. Capture at frame 700 with
  `AURORA_FPS_LIMIT=60`, in `--language en` and `fr`.
- **`AURORA_DEMO_COLLISIONS=1|chat`**: MeanCollisionAlert as the simulator
  sends it (`world/collisions.rs`), at 60 frames a second: the "Bumps,
  pushes & hits" window opens at frame 600, a resident bumps the agent at
  640, a second one pushes it with a script at 760, a third one hits it
  with a scripted then a physical object at 880, and at 1000 the first one
  bumps it again (that row comes back to the top with the new time) while
  the second one hits it with an object and sends a type the viewer does
  not know (left out, logged as `collision of unknown type`). `chat` also turns on "Collisions in nearby
  chat" for this run (Preferences › Interface is not changed): one line
  per collision, the name a link to the profile. Capture after frame 1000
  with `AURORA_FPS_LIMIT=60`.
- **`AURORA_DEMO_LINK=stall:<frame>[:<seconds>],lost:<frame>`**: the link
  with the agent's region as the user feels it (`ui/link_health.rs`).
  `stall` makes the demo region silent from that frame for that many
  seconds (20 by default, counted at 60 frames a second): after 5 s the
  pill "Waiting for the region (n s): the connection is unsteady" shows
  under the top bar and counts, then goes away when the silence ends.
  `lost` ends the session at that frame as a lost link does (silence of
  100 s, event queue given up, region gone): the login screen says so and
  the next login goes back to the region and the spot the user was in
  (`link lost: "Reconnect" goes back to uri:…` in the log; the demo has no
  grid, its login only logs `demo login: reconnect to uri:…`). Both can be
  joined with a comma.
- **`AURORA_DEMO_RESTART=<seconds>[:options]`**: at frame 600 the demo
  region announces that it restarts in `<seconds>` (AlertInfo
  RegionRestartSeconds): the alert card drops in at the top third of the
  view and trembles, the restart sound plays, the taskbar icon flashes 5 s,
  a line goes to the nearby chat and the card is pinned in the bell
  without a toast. The count runs in real time; it turns orange at 1:00,
  red at 0:30 (second shake, sound) and becomes the final count at 10.
  Options, joined with `:`: `fast` plays the mockup's fast mode (5 minutes
  in about 18 s); `still` stops the count and freezes the border's breath
  at its mockup value, for screenshots (the bar then empties over 5
  minutes, as in the mockup's states); `manager` adds the managers' line
  ("Cancel the restart" with its confirmation: the demo's simulator
  cancels at once, the card turns green then leaves); `min` reduces it to
  its pill, `closed` closes it (it comes back as a pill at 30 s), `lm` opens
  the landmark menu, `bell` opens the bell's list, `toast` makes an IM
  arrive, `dnd` turns do not disturb on (a pill only), `opensim` plays
  OpenSim's notice (`/RESTART_X_SECONDS`, its sound), `legacy` shows
  Firestorm's yellow box with the view shake. Capture with
  `AURORA_FPS_LIMIT=60` at frame 700 (the drop and the shake are over).
  Preferences › Interface › Region restart (alert, shake: the alert / the
  view / none, the channel announcement) and "Reduce animations" change it
  live.
- **`AURORA_DEMO_TOASTS=1|bell|click|minimized`**: a friend goes offline at
  frame 700 and comes back at 760 (two presence toasts, round picture with
  its status dot), two IMs from the same resident arrive at 820 and 880 (one
  card, "(2)"), a group IM at 940 (group toasts are forced on for the
  scenario). The Conversations button of the bottom bar shows the unread
  IMs and group chats in the red counter (5 at frame 1000), not the nearby
  chat lines.
  The friends announced at login get no toast; their nearby chat line only
  shows with Preferences › Chat › Toasts › "Friends online / offline in
  nearby chat" (off by default). The window closed, the Conversations
  button flashes in violet at each IM (eight soft rises over four seconds,
  started again by the next one), then stays tinted until the window opens. `1` opens the bell
  list at frame 1100: by default the messages only toasted and nothing is
  listed (Preferences › Chat › Toasts › "Keep messages under the bell",
  off). `bell` keeps them under the bell: the two conversations are listed,
  never the presence notices. `click` (window closed) and `minimized`
  (window minimized at start: its bar flashes at each IM instead of the
  button, then stays tinted while they wait unread) also keep them, and click the first IM card at frame 1250: the
  Conversations window opens or expands, comes to the front on that
  conversation, and its card leaves the list. Capture with
  `AURORA_FPS_LIMIT=60`, at `850,900,1160` for `1` and `bell`, and
  `835,1240,1310` for the click modes. Add `AURORA_DEMO_SOUND=1` to hear the
  existing sounds.
- **`AURORA_DEMO_CONV_BUTTON=1|mute|still|menu`**: the Conversations window
  closed, Tess says something in nearby chat at frame 700 and Nova writes an
  IM at 900. The demo's group chat already lit the chat island's button
  (faint tint and light outline, as a minimized conversations bar); each of
  the two messages flashes it again (eight rises over four seconds, the
  outline widening with the wash). `mute` leaves nearby chat out for the run
  (never saved): 700 stays at the tint, 900 flashes. `still` turns Reduce
  motion on for the run: the button holds the top of a rise for the four
  seconds instead of rising and falling. `menu` right-clicks the button at
  700: "Open conversations", then "Don't flash for nearby chat" (unchecked).
  Capture with `AURORA_FPS_LIMIT=60` at `690,715,730,915,1200` (715 and 915:
  the top of a rise; 730: its bottom), `760` for `menu`.
- **`AURORA_DEMO_DELIVERY=1|several|open|history`**: the Marketplace inbox
  (Received items) appears at frame 600 with an earlier purchase, which is
  not announced; at 700 a new delivery lands in it the way the server's
  inventory update adds it, with no offer (#752). `1`: one "Marketplace
  delivery" card unrolls from the bell with the purchase's name and "Show in
  inventory". `several`: three land at once, one card counts them and lists
  their names in order. `open`: the button is pressed at 900, the Inventory
  window shows the Received tab with the delivery selected and the card
  leaves the list. `history`: the card is closed at 900 and the bell's list
  opens on History, where the entry reads "Read" with "Show in inventory".
  Capture with `AURORA_FPS_LIMIT=60` at `760` for `1` and `several`, `960`
  for `open` and `history`. Turning off Preferences › Inventory › Received
  items › "Notify Marketplace deliveries" leaves only the Received badge.
- **`AURORA_DEMO_MENTION=list|names`** (#980): the nearby chat bar gets the
  keyboard at frame 540. `list` types `Hi @` at frame 550: the residents
  within chat range (20 m) are listed above the bar, nearest first, the
  first highlighted; capture frame 620. The arrows choose, Tab or Enter
  write `@Name ` (the display name), Escape closes until the letters change;
  sent, the name becomes `secondlife:///app/agent/<id>/mention`, drawn as
  the resident's name in the line. `names` turns the name prediction on for
  the run (never saved), types `Hi ` then, at frame 560, the first three
  letters of the nearest resident's user name (the log's `demo mention:`
  line): the rest of their first name and a space follow, selected; capture
  frame 620.
- **`AURORA_DEMO_KEYWORDS=1|prefs`**: keyword alerts on for the run, never saved
  (the two keywords of `demo-keywords-words`, "party, builders" in English,
  "fête, constructeurs" in French; colour change and sound on), with the
  Conversations window open on the nearby chat. A resident says a line
  holding a keyword at frame 700, another sends one in an IM (760), a third
  writes one in the Aurora Builders group chat (820), an object sends one in
  an IM (880), and a line without one is said (940); capture with
  `AURORA_FPS_LIMIT=60` at frame 1010. The four keyword lines are drawn in
  orange in every chat style and play the keyword sound (silent in captures unless
  `AURORA_DEMO_SOUND=1`); with the system notifications on, each is told as
  a "Keyword alert". `AURORA_DEMO_OPTIONS=3:keywords` opens Preferences ›
  Chat on the group (it shows the saved settings, not the run's). `prefs`
  keeps the settings of Preferences › Chat › Keyword alerts, to try them by
  hand, and sends the same five lines again every 1200 frames.
- **`AURORA_DEMO_ATTENTION=1|front|capture`**: the window goes to the
  taskbar at frame 700, then an IM (760), a mention in nearby chat (820), a
  mention in a group chat (880), an object's message (940), a teleport offer
  (1000) and a script dialog (1060) arrive. Each but the script dialog
  flashes the taskbar button ten times, which then stays lit (Preferences ›
  Chat › Taskbar and system notifications; the script dialog's switch is off
  by default); the system notifications are turned on for the scenario: one
  Windows notification (on Linux, one from the desktop's notification
  service) per arrival, a click brings the window back on the conversation,
  the nearby chat or the bell's list. The log says `taskbar flash` and
  `system notification shown (<what a click opens>)`, never the text.
  `front` keeps the window in front: nothing flashes and nothing is shown.
  `capture` brings the window back at frame 1150 with the bell's list open:
  capture with `AURORA_FPS_LIMIT=60` at frame 1300. The first notification
  writes the application's name and icon under
  `HKCU\Software\Classes\AppUserModelId\Aurora.Viewer` (what Windows shows
  above it). On Linux the log names the notification service and its
  capabilities at the first one; a tiling compositor that ignores the
  minimize (Hyprland, Sway) keeps the window focused, so nothing is shown
  unless the window is moved away from the screen first.
  `AURORA_DEMO_OPTIONS=3:attention` opens the switches.
- **`AURORA_DEMO_EXIT=1|cancel|logout|quit`**: opens the modal at frame 650.
  `1` leaves it interactive; the other modes make the choice at frame 800.
  Take screenshots at `740,900` to compare the blurred background then the
  resumed session or the return to the login. `quit` ends the process at
  frame 800 (capture only 740, without `AURORA_CAPTURE_EXIT`). Combine with
  `AURORA_DEMO_LOGIN=empty` for the login screen: Log out is
  disabled. The choices are simulated offline, with no login request.
- **`AURORA_DEMO_VOICE_PREFS=<modes>`**: comma-separated `avatar`, `raw`
  (AGC, AEC and noise suppression off; high-pass remains enabled), `no-lips`,
  `no-indicators`, `no-self`, `no-dot`, `no-call-visuals`, `no-conversation`.
  Combine with `AURORA_DEMO_VOICE=speaking|cycle` and `AURORA_PROFILE=1` for
  matched visual/performance comparisons, or `AURORA_DEMO_CALL=group` to
  check that hiding world indicators leaves the call's controls intact.
- **`AURORA_DEMO_TOOLBOX`**: `1` opens the Dev Toolbox (Advanced › Dev
  Toolbox, Ctrl+Alt+D); `edit` opens it in Customize (grips, "+ Add a
  tool"); `<target>:<tool>` chooses the target (0 nearest avatar, 1
  selection, 2 under the cursor, 3 my avatar) and presses the tool once the world shows
  (frame 600), its text in "Send a diagnostic": `0:J1` for the worn items of
  the avatar within 3 m (with `AURORA_DEMO_POS=134.5,128`,
  `AURORA_DEMO_CROWD=4`, `AURORA_DEMO_ANIMESH=1`), `0:J4` for the system
  configuration, `0:N3` for the loads, `0:J9` for the skeleton of the
  animesh the nearest avatar wears (with the same position, crowd and
  `AURORA_DEMO_ANIMESH=1`; its text also goes to the log after `demo dev
  toolbox J9:`: one worn animesh on point 6, 5 joint overrides from mesh
  `d0d0a111-…0100`, the demo animation playing). Prefer J1 to J2 in automated runs:
  J2 copies to the clipboard of the PC. Capture after frame 1000 when names
  are asked (3 s).
- **`AURORA_DEMO_OPTIONS=<page>[/<tab>][:<what>]`**: opens the preferences
  on a page by its name (`aurora`, `graphics`, `camera`, `avatars`, `chat`,
  `sound`, `media`, `privacy`, `layout`, `appearance`, `accessibility`,
  `confirmations`, `inventory`, `controls`, `shortcuts`, `commands`,
  `system`, `network`) and one of its tabs counted from 0 (`chat/2`: Chat ›
  Keywords). The numbers of the window before its reorganisation still work
  (`0` Graphics, `1` Layout, `2` Appearance, `3` Chat, `4` Mouse and
  keyboard, `5` Appearance › Colours, `6` Network and cache, `7` Sound and
  voice, `8` Shortcuts, `10` Camera, `11` Commands, `12` Privacy, `13`
  Aurora, `14` Accessibility); `9`, the former Debug page, opens the Dev
  Toolbox instead. `<what>`:
  - `altgr` on `shortcuts`: at frame 600, captures AltGr + F9 and AltGr +
    Shift + F9 in the Dev Toolbox's two slots, dispatches each through the
    real shortcut handler and restores the demo's saved bindings. Both
    `demo altgr shortcuts` log lines must say `captured=true opened=true`;
    the toolbox stays open, the shortcut page shows its ordinary binding.
  - `group-titles` on `avatars/1`: reveals the group-title dropdown with
    All except mine selected. The separate hide-own-title switch is absent;
    check None / Only mine / All except mine / All and the French labels.
  - `recent` on `inventory`: reveals "Recent tab shows" behind the Recent
    tab's cog, on "Since last logout"; `recent-period` sets it to a week
    first, its days and hours beside the choice. The inventory window's own
    preferences show the same choice under its Recent tab switch.
  - `search=<text>`: the window's search filled in. Every row whose name,
    help or choices have a word starting with each word typed shows under
    its page and group, its control live, its "go to" arrow and the group's
    "Open the page" lighting it on its page; the sidebar counts the hits per
    page and dims the others. Accents and case do not count (`ombre` finds
    "Ombres", not "Nombre"); a shortcut is found by its keys (`Ctrl+M`), a
    command by its name (`gtp`). `zzzz` shows the empty result.
  - `modified`: the settings that differ from their defaults, all pages
    together, with "Reset them all" (and its Undo).
  - `cogs`: every cog open (the rows behind the cog of a row: the twelve
    effects of Aurora's water, the eight of the depth of field…), for
    captures; each opened block has its own darker face.
  - `reset`: the page reset at once, with its Undo at the bottom; the chat
    says it was reset once the Undo is gone (8 s) or the window closed.
  - `restore` (with `system`): the confirmation of a settings restore, for
    a backup of the demo's own profile (preferences, windows' layout, skin,
    accounts' files when they exist); "Restore" puts them back as a real
    backup file would, and the chat says so.
  - the rows of the former filters, lit a moment and scrolled to:
    `0:tonemap` (Graphics › Image › Tone mapping), `0:occlusion` (Light ›
    Indirect light, its cog opened), `0:bloom`, `name-tags`, `attention`
    (Chat › Notifications), `keywords`, `gifs` (Chat › Nearby chat › GIFs,
    its keys explained), `7:voice`, `7:processing`,
    `7:indicators` (Sound and voice › Voice), `13:animations` /
    `13:animations-sync` (Aurora › Behaviour, Legacy / Synchronized clock).
    `8:<text>` searches the shortcuts (`8:Ctrl`, `8:HUD`).

  What to look for: the sidebar's sections, each page's tabs, the rows in
  two columns (one under 660 points), a violet dot before a modified row
  (under the pointer, its reset), the cogs with their number of rows,
  greyed and dimmed while their row is off. Combine with
  `AURORA_DEMO_LOGIN=empty` and `--language fr|en` for preference captures.
  `controls` shows the independent "Fly by holding Up" and "Land by holding
  Down" switches, both on by default. Try all four combinations: Up still
  jumps or ascends, Down still crouches or descends, and the Fly button and
  shortcut still start/end flight. With automatic landing off, descending
  onto the ground must retain flight. Verify the choices after a restart.
  With automatic takeoff off, the first permitted ground jump starts without
  the pre-jump wait, even when Quickjump is off; holding Up keeps repeating
  jumps. Releasing Up restores the usual landing animation handling. Check
  that seated avatars and jump restrictions still prevent jumping, and that
  flight and the swimming/water-exit controls keep their existing behavior.
  `2:text-size` opens Appearance and cycles the text-size preference through
  0.8, 1.0, 1.4, 2.0 and 1.0 every 180 frames after frame 600. Capture
  frames 700, 880, 1060, 1240 and 1420 to compare the preview, navigation, window titles,
  world labels and conversations, including inline emoji. Changes use the
  normal live preference path and only affect the demo profile.
  The Preferences window must retain its size throughout the cycle,
  including after a manual resize to a shorter height. The sidebar scrolls
  independently of the settings; the footer keeps "Reset everything…", the
  page's reset and Close. `0` shows the Presets row at the top of Graphics:
  with `AURORA_GRAPHICS_PRESET` its level is lit, otherwise (the defaults)
  no segment is and "Custom" follows the row. The microphone popup
  exposes camera/avatar ears and all five noise levels; its processing
  shortcut opens Sound and voice › Voice. `AURORA_DEMO_VOICE=cycle`
  exercises the world indicators and GPU lip sync; disable each preference
  and check the other indicators (call participants and nearby people)
  remain functional. Private/group call spatial audio and real microphone
  quality require human grid testing.
  `1:scale` profiles live UI scaling: from frame 600, resize from 1.0× to
  1.5× and back over 600 frames, repeating through the same setting action
  as the slider. Combine with `AURORA_PROFILE=1`, `AURORA_FPS_LIMIT=60` and
  captures at frames 600, 900 and 1200. It does not take the native pointer.
  `1:scale-keys` reproduces issue #853: reset the scale at frame 600, use
  Ctrl+Plus/Equals to reach 120%, then Ctrl+Minus to reach 80%, and toggle
  window snapping after each change. Capture frames 700, 800, 960 and 1040:
  each pair must keep the same scale before and after the unrelated option
  changes. The log compares the saved and displayed scales. No native input.
  With `AURORA_DEMO_LOGIN` (for example `empty` or `cards`) the preferences
  open over the login screen, as its top right cogwheel does: every page
  works before login, and Chat › History says that the folder
  belongs to an account.
- **`AURORA_DEMO_COLORBLIND=protan|deutan|tritan|mono[:strength]`**: the
  interface adapted to a colour vision deficiency for this run, as
  Preferences › Accessibility › Colour blindness does, without touching the
  saved settings (`:0.5` for half the strength). `14` of
  `AURORA_DEMO_OPTIONS` opens that page, with its preview of the status
  dots. What to look for (compare with a capture without the variable):
  `protan` / `deutan` turn success and online blue, `tritan` turns warnings
  orange and links sky blue, `mono` turns everything grey with the
  meaningful colours at clearly different lightnesses; the rest of the
  interface keeps its look. Whatever the mode, the status dots differ by
  their shape (a disc online, a crescent away, a barred disc in do not
  disturb, a hollow ring offline): combine with `AURORA_DEMO_CONV=bubbles:busy`,
  `AURORA_DEMO_PEOPLE=rows` or `AURORA_DEMO_CONTACTS=friends`.
- **`AURORA_DEMO_CONTRAST=1|0`**: the high contrast palette for this run,
  as Preferences › Accessibility › Vision does, without touching the
  saved settings (`0` forces it off on a profile that has it on). What to
  look for: opaque black windows and bars (no glass), white text, light
  grey secondary text, a light grey border around buttons, fields, windows
  and the bar islands, black text on the light violet accent; combine with
  `AURORA_DEMO_OPTIONS=14` (the Accessibility page) or
  `AURORA_DEMO_COLORBLIND`, which applies after it.
- **`AURORA_DEMO_COLORS=accent:<hex>,surface:<hex>,text:<hex>`**: the
  interface colours for this run, any of the three, as Preferences ›
  Appearance › Skin › Interface colours does, without touching the saved
  settings (for example `accent:#14B8A6` for a teal accent, or
  `accent:#EC4899,surface:#3A2433,text:#FCE7F3` for a pink interface).
  What to look for: the accent on the main buttons, switches, sliders,
  selection and the 3D selection outline; the bars, fields and button
  faces in the background's hue; the secondary text in the text's hue;
  text that still reads on its background. `AURORA_DEMO_CONTRAST=1`
  ignores them. Combine with `AURORA_DEMO_OPTIONS=appearance/2` (the page).
- **`AURORA_DEMO_COLORPICK=1`**: at frame 300, opens the compact colour
  picker (`widgets::color_button`) of the first colour swatch
  drawn. To be combined with `AURORA_DEMO_OPTIONS=5` (Appearance ›
  Colours): the panel opens under "My messages", with the saturation / value field, the
  hue bar, the new colour above the old one, the
  R G B and hexadecimal fields, and the eyedropper.
- **`AURORA_DEMO_GLYPHS=1|2`**: eight residents in two rows in front of
  the avatar (the second row flying), a cube with floating text on the right
  and the Conversations window on local chat. The group title, display name
  and message of each one use one family of characters: Georgian and
  Armenian (the title "ღAurora Friendღ" of issue #127), mathematical
  letters (bold, script, fraktur, double-struck), symbols and
  enclosed characters, Cyrillic and Greek, Japanese / Korean / Chinese, Arabic
  and Hebrew, other scripts (Canadian syllabics, Cherokee, Devanagari,
  Thai, Tamil, Tibetan…), emojis. `2`: same messages in reverse
  order (the default chat window shows the last four).
  What to see: no square in the labels, the floating text,
  the chat and its list; all the characters of a line sitting on the same
  baseline. Compare with `AURORA_FONT_FALLBACK=0` (squares everywhere
  except Cyrillic, Greek, emojis and a few symbols). Visible limits: flags,
  skin tones and ZWJ sequences not composed; emojis
  monochrome outside the chat. On a system without the expected fonts
  (see `ui/fonts/fallback.rs`, verified on Windows 11), the squares
  remain for the missing scripts; the log lists the fonts
  found (`system fallback fonts: …`).
- **`AURORA_DEMO_GLYPHS=rtl`**: the same scene in right-to-left text
  (`ui/bidi.rs`): an Arabic sentence with an emoji, a Hebrew one, an English
  sentence with Arabic in it, Arabic with times, numbers and brackets,
  Arabic-Indic digits, Hebrew with "Aurora Viewer" in it, a long Arabic line
  that wraps, an Arabic line with a link; Arabic and Hebrew names and titles,
  Arabic floating text on the cube. What to see: in the name tags, the
  transcript, the chat bar and the bubbles, the first word of an Arabic or
  Hebrew sentence on the right and the words read from right to left; an
  English sentence still from the left with its Arabic words read from the
  right; "8:30", "42" and "١٢٣٤" read from the left; brackets around the
  words they hold; in the transcript, a message read from the right starts
  at the right edge, its link and its emoji in reading order, a wrapped line
  goes on on the row below from the right; a drag selects and Ctrl+C copies
  in reading order. The residents' display names are known before they
  talk, so each line carries its sender's complete name ("ذئب بنفسجي
  (glyph1)"); in a message header, a name read from the right has its first
  word on the right. `rtl2`: the lines in reverse order, the two-word Arabic
  and Hebrew names among the six the chat bar shows; with
  `AURORA_DEMO_BARS=legacy AURORA_DEMO_CHAT_LINES=legacy` the Conversations
  window stays closed and the lines show above the chat bar in the one-line
  look, the sender's picture inside the line: what to see, the picture
  first, then the whole name in its own order (never under the picture,
  never split by a gap), then the colon and the message. The Arabic and
  Hebrew letters need a system font that has them (on Linux, DejaVu Sans or
  Noto Sans Arabic / Hebrew).
- **`AURORA_DEMO_CONV_REORDER=1|hold|out|nearby`**: with
  `AURORA_DEMO_CONV=aurora` (also `aurora:compact`, `aurora:narrow` or
  `legacy`), synthetic pointer events drag the first conversation after the
  next of its category (pressed at frame 701, moved until frame 720, let go
  at frame 760). Captures simulate logical focus without activating the
  native window or taking the user's keyboard. Capture frames 690, 740 and
  800 at `AURORA_FPS_LIMIT=60` for the initial order, the ghost under the
  pointer with the entries moved apart where it lands, and the completed
  move. `hold` never lets go: the ghost and the gap stay. `out` drops the
  first conversation at the left of the Conversations window and `nearby`
  the nearby chat (#910): its own window opens where the ghost was let go,
  the ghost shows the Detach mark while it is outside the window. With
  `nearby`, the chat island's button then shows and hides the nearby chat's
  window alone. Manually check upward/downward moves within private
  messages, groups and conferences; reject drops across categories and on
  pinned entries; a drop inside the window but off the list does nothing;
  Escape cancels. A small pointer movement remains a click. Check
  filtering, edge scrolling, close buttons, menus and switching chat
  styles. Order lasts while those conversations remain open; closing and
  reopening a conversation appends it again. No grid is needed.

- **`AURORA_DEMO_CONV_GIVE=away|over|many|drop|give`**: with `AURORA_DEMO_CONV=aurora`
  (also `aurora:compact`, `aurora:narrow`, `bubbles` or `bubbles:legacy`), an inventory drag starts at
  frame 700 and is never released. `away` leaves the pointer outside the
  window: the friend's messages fade behind the gift and "Drop here to
  give to <name>" in the muted ink. `over` brings the pointer onto the
  conversation: the violet outline of the drop zone and the hint in the
  ink. `many` carries three entries: "3 items" under the hint. `drop`,
  with `AURORA_DEMO_INVENTORY=1`, drops seven of its entries on the
  conversation at frame 720: the give confirmation opens, the resident by
  name, five entries and "and 2 more", the no-copy object tagged with the
  amber warning, the no-transfer one greyed and tagged with the line
  saying it is left out (its reason on hover), then Cancel / Give. Capture at
  frame 780 with `AURORA_FPS_LIMIT=60`. `give` answers that confirmation
  with Enter at frame 740: the conversation then shows one muted notice
  per given root, "Inventory item « … » offered" (or "Inventory folder"),
  without counting as unread (#736). The field has no attach button.
  Bubbles and Legacy use the same drop and confirmation path, including
  detached conversations; their feedback is the violet outline and copy
  cursor, without the workspace's fading hint.
  Manually: drag an item from the inventory onto a private conversation
  (the Share confirmation opens), onto a group's (no hint, nothing given),
  and Escape during the drag (the messages come back). No grid is needed.

- **`AURORA_DEMO_CONTENTS_DROP=world|panel[:ctrl][:drop]`**: with
  `AURORA_DEMO_INVENTORY=1` and `AURORA_DEMO_ACTIONS=1`, a selection of
  four entries (a notecard, a script, a sound and an object) is carried
  from frame 700, without a press. `world` puts the pointer on the yellow
  action object 976 (yours, modifiable): without `:ctrl` the object in the
  selection needs Ctrl, so nothing is outlined and `:drop` says "Hold Ctrl
  over an object…"; with `:ctrl` the box gets the violet outline and
  `:drop` (frame 720) puts the four entries into its contents, the script
  stopped. `panel` puts the pointer on the Contents tab: add
  `AURORA_DEMO_BUILD=move,object:976` and `AURORA_DEMO_BUILD_TAB=contents`
  (also useful with `world` to see the contents arrive); the list gets its
  violet outline and `:drop` adds the four entries, the script running
  unless `:ctrl`. Capture at frame 715 (hover) or 800 (drop) with
  `AURORA_FPS_LIMIT=60`: "Contents (3 items)" becomes "(7 items)".
  Manually: select several items (Ctrl+click) or a folder and drag them
  onto an object you own with and without Ctrl, onto someone else's (a
  no-transfer item refuses the whole drop), and onto the Contents tab.

- **`AURORA_DEMO_GIVE=profile|avatar[:over|:drop]`**: with `AURORA_DEMO_INVENTORY=1`, seven of
  its entries are carried from frame 700, without a press. `profile`, with
  `AURORA_DEMO_PROFILE=wolf` (or any other resident), leaves the pointer
  beside the profile window: the whole window fades behind the gift and
  "Drop here to give to <name>" in the muted ink, "7 items" under it.
  `profile:over` puts the pointer on the window: its violet outline and the
  hint in the ink. `profile:drop` releases there at frame 720: the give
  confirmation of the conversations opens for that resident. `avatar` puts
  the pointer on the other avatar nearest the middle of the view: it and
  what it wears get the violet outline of the photo's chosen subjects;
  `avatar:drop` releases there at frame 720 and opens the confirmation. For
  a readable avatar capture use `AURORA_DEMO_CROWD=12`,
  `AURORA_DEMO_POS=129,134` and `AURORA_DEMO_CAMERA=close,1110,420,20`;
  capture at frame 715 (`over`) or 760 (`drop`) with `AURORA_FPS_LIMIT=60`.
  The demo crowd is drawn as a grid: compare with a capture without the
  variable to see the outline. Manually: drag an item, a folder or a
  selection onto another resident's profile (any tab, both styles), onto
  our own (nothing), onto an avatar, onto what it wears, onto an avatar
  behind a wall (the wall wins), and Escape during the drag. No grid is
  needed.

- **`AURORA_DEMO_SNOOZE=<seconds>[,auto][,ask]`**: from the 10th second in
  world, Tess writes in the Aurora Builders group chat every 6 seconds
  ("Group update number n"). The Snooze of a group chat (its head's button
  beside the close one, or its entry's right-click menu) lasts `<seconds>`
  instead of the minutes of Preferences › Chat › Group chat snooze, so
  that its end can be watched; `auto` snoozes that chat by itself at the
  14th second; `ask` asks the duration first, as "Ask the duration at each
  snooze" does, without changing the saved setting: the dialog counts
  minutes and proposes `<seconds>` rounded up (1 for 20). The
  conversation leaves the list, its
  messages are dropped (`snoozed group chat …: message dropped` in the
  log), and the first one after the snooze opens it again with the
  new-session sound (`group chat snooze over, reopening`). With
  `AURORA_DEMO_SNOOZE=20,auto`, `AURORA_DEMO_CONV=aurora:group` and
  `AURORA_FPS_LIMIT=60`: the chat at frame 780, gone at 1500, back at
  2280 with the message of the 34th second. With `ask`, the dialog shows
  from frame 840 until it is answered.
- **`AURORA_DEMO_THUMBNAILS=bar|top|off`**: where the conversations'
  thumbnails stand for the run (`ui/chat/thumbnails.rs`), Preferences ›
  Chat › Conversation thumbnails left alone. `bar` (the default): a glass
  island between the chat island and the dock, or in the Legacy bar after
  the chat field; `top`: an island at the top right of the windows' area;
  `off`: none. One square per open conversation (picture, insignia or
  conference glyph) with the red unread counter; with more conversations
  than room (`AURORA_DEMO_CONV=many`), a "+N" square lists the others and
  the unread ones stay in sight. With `AURORA_DEMO_FLASH=1`, Tess's
  thumbnail flashes from the 10th second (eight rises, then a faint mark
  until her conversation is opened).
- **`AURORA_DEMO_FLASH=1`**: from the 10th second, Tess writes to us every
  8 seconds ("Are you there? (n)"), while the Conversations window is
  open. With another conversation shown, her entry (the column in
  Aurora / Compact, the tab in Bubbles / Lines) takes a violet wash that
  rises and falls softly eight times over four seconds, then stays faint
  until her conversation is opened; each message starts it again.
  Preferences › Chat › Flash unread conversations turned off: no wash, the
  unread count only. Combine with `AURORA_DEMO_CONV=aurora:group` (or
  `bubbles:group`) so that her conversation is not the one shown.

- **`AURORA_DEMO_CONFERENCE=1|<seconds>`**: every 20 seconds (or every
  `<seconds>`) a conference invitation arrives, from Tess Touch (not a
  friend) then from Orion (a friend) in turn, each in a new conference.
  Settings off: each opens in Conversations › Conferences. With Preferences
  › Chat › Groups and blocking › Ignore conferences (or Online status › Ad
  hoc conferences › Ignore all and exit automatically) on, none opens;
  "Report ignored conferences" adds a nearby chat line naming the inviter,
  "Except from my friends" lets Orion's through and still ignores Tess's.
  The log says `ignoring conference (ad-hoc) chat from <name>` for each.

- **`AURORA_DEMO_CONV_LINE_MENU=1`**: with `AURORA_DEMO_CONV=aurora:group`
  or `aurora:compact:group`, right-click a visible message at frame 702.
  Combine with `AURORA_DEMO_SNOOZE=300` (without `auto`) and `AURORA_FPS_LIMIT=60`:
  captures at frames 780 and 1500 show the same highlighted target while
  new group messages arrive below the paused viewport. Inline place, avatar,
  command and preview menus hold and highlight their containing message too.
  The workspace's
  New marker moves below the messages present when the menu opens and
  stays before the first arrival. Escape, an outside click or a menu action
  closes the menu, removes the highlight and resumes following if the
  conversation was at its end beforehand. Repeat manually after scrolling
  up: closing keeps the older reading position. Also check the sender menus
  in Bubbles and Legacy, and Copy/Quote/Ignore against the original target.

- **`AURORA_DEMO_CONV_SCROLL=up|new`**: with `AURORA_DEMO_CONV=aurora`,
  `aurora:compact`, `bubbles` or `bubbles:legacy` (the friend's
  conversation), forty earlier messages come before the scenario's lines and
  the conversation opens on its top. A button fades in over the bottom of
  the lines, above the field: "Jump to the latest messages" with `up`;
  with `new` the friend writes three messages once the top is shown and it
  reads "3 new messages". A click brings the last message back against the
  field and the button fades out; a message written then shows at once,
  the lines following the end again. Scrolling back down by hand also hides
  it once the end is less than half the height in sight away (with nothing
  new).

- **`AURORA_DEMO_FRIEND_LOCATION=allowed|denied|offline|altitude|join-map|join-contact|join-fly|join-fly-blocked|join-fly-late|join-fly-custom`** (#999): removes
  Nova's nearby object and coarse location at frame 60. The online friend
  grants us map rights in `allowed`, withholds them in `denied` (we still
  grant ours), and is offline in `offline`. Combine with
  `AURORA_DEMO_CONV=aurora`, `AURORA_DEMO_CONV_MENU=im` and a frame-780
  capture: Teleport To and Show on map are enabled only in `allowed`,
  while Zoom/Go to/Follow stay disabled. Clicking Show on map resolves
  the friend in Boreal Lagoon through TrackAgent/FindAgent; Teleport To
  uses the same lookup once, then the normal teleport path. Also inspect
  `AURORA_DEMO_PROFILE=friend` (the overflow Map action) and the friend
  sheet opened from the Contacts tab. On the grid, the human checks an
  online friend in a distant region, grant/revocation in both directions,
  map opening from the menu/profile/sheet, and one actual teleport.
  FindAgent supplies only global X/Y: the target's matching local height
  is used when available; otherwise Z defaults to 0 m (ground), never our
  current height. Both map layouts explain unavailable friend altitude;
  coarse updates refine a local target's height in 4 m steps. Set Z for a
  remote sky platform; the full-frame map retains an explicit height while
  following the same friend. When a map/contact teleport starts without a
  chosen Z or known friend height, arrival in the destination region arms
  one local TP To using new target data. Old cache and coarse unknown Z
  values (0/1,020 m) cannot trigger it. Movement, Stop/Escape, a new teleport,
  map/height edits, revoked rights, failure or timeout cancel it; accepted
  teleport invitations do not arm it. The wait is bounded to 90 seconds
  before arrival and 30 seconds for target data after arrival.
  `altitude` is the allowed remote scenario with
  our avatar at 3,000 m: opening the friend's map must show Z=0 and the
  unavailable-altitude note. On the grid, the human checks a departure from
  a high skybox to a friend near ground level and a manually chosen Z.
  `join-map` and `join-contact` start a map/contact teleport at frame 700/720,
  arrive in another region, then deliver the friend's object at 2,400 m
  after a 700 ms delay. Capture at frame 1,450: the second local teleport
  must put us at 2,400 m and update the map height, without a third teleport.
  On the grid, test friends near ground and on a skybox, delayed appearance,
  movement/cancellation and permission revocation. Accepting a teleport
  offer or choosing Z explicitly must keep the supplied landing point.
  `join-fly` starts grounded with Fly After Teleport ignored for this demo,
  then supplies a hovering friend at 2,400 m and a flying local-reply flag:
  the local jump must complete
  before flight turns on and holds that height. The scoped log must show
  arrival at 2,400 m before `friend join: flight enabled after local arrival`.
  `join-fly-blocked` supplies a no-fly region:
  takeoff is not enabled, but the second jump must still happen.
  Check that an already flying user remains flying, a standing skybox friend
  does not enable flight, and accepting a later invitation keeps its own flags.
  `join-fly-late` supplies Standing first and a local reply without the flying
  flag, then a hover signal 400 ms after the jump and the allowed landing
  parcel another 400 ms later. The cached parcel's bitmap covers a different
  position and must neither approve nor reject flight. Scoped logs must show
  arrival, the delayed hover, waiting for the landing parcel, then flight
  activation. Some height is lost before the delayed replies; flight must
  then hold altitude. A join observes new flight/parcel data for at most five
  seconds after arrival; standing/unknown signals alone never enable flight.
  `join-fly-custom` supplies no native locomotion signal (Typing represents
  an unclassified AO motion), but fresh physics with no vertical movement
  and a nonzero plane at 20 m while the target hovers at 2,400 m. That plane
  must not be confused with actual support beneath the avatar's feet.
  Flight must start after a sustained 300 ms
  hover observation following arrival. The log identifies the inferred hover.
  A standing skybox target with a contact plane, a seated avatar, missing
  physical data, a jump or a fall must never trigger this fallback.
  Also test a friend whose height is already known, through Contacts and
  the map: the direct teleport must retain the friend for arrival flight,
  without adding another hop. A matching AgentMovementComplete is enough
  to confirm arrival; flight must not depend on a separate TeleportLocal.

- **`AURORA_DEMO_CONV_MENU=local|im|group|conference`**: with an
  `AURORA_DEMO_CONV` scenario (`aurora`, `bubbles`, `legacy`…), a right click
  at frame 701 on the first conversation of that kind in the list (the
  workspace's column or the tabs) opens its menu: capture at frame 780 with
  `AURORA_FPS_LIMIT=60`. `im` (`AURORA_DEMO_CONV=aurora`): Close and Voice
  call, the resident's menu of a name, then Invite to group, Show on map,
  Share and Pay; `group` (`aurora` or `bubbles:group`): Close, Voice call,
  the transcript, Group profile, Activate group (greyed for the active
  one), Stop receiving its chat, Leave group; `conference`
  (`aurora:conference`): Close, Hang up during its call, the transcript;
  `local`: the transcript only. The transcript rows are greyed in the demo
  (no transcript folder).
- **`AURORA_DEMO_CONV_POPOUT=1|option|nearby|nearby-out|dock|dock-hold`** (#717, #995, #1026): with an `AURORA_DEMO_CONV`
  scenario (`aurora`, `aurora:compact`, `bubbles` or `bubbles:legacy`), at
  frame 640: `1` pops the friend's IM (Nova) and the Aurora Builders group
  out into their own windows, at the left of the screen, one above the
  other; `option` turns Preferences › Chat › "Open conversations in their
  own window" on for the run (not saved) and opens Aurora Builders as a
  notification would, in its own window at the top left, the
  Conversations window keeping its conversation. Capture at frame 760 with
  `AURORA_FPS_LIMIT=60`. What to see: each window with its head, lines and
  field, the "Attach to Conversations" button in its title bar; the column
  (or the tabs) hides their entries until reattached. Drag a detached
  window's title onto the expanded Conversations window: its violet outline
  marks the target, and release reattaches and selects the conversation.
  Dragging outside, resizing or selecting messages must not reattach.
  `dock` uses the real pointer events at frames 700–741 to drag the friend's
  title onto the container; capture frame 780 for the attached result.
  `dock-hold` keeps the button down; capture frame 730 for the target outline.
  Contacts uses the same title drop; closing Contacts or nearby chat restores
  its pinned entry without opening the container. The Conversations window shows another conversation and its
  title bar a "Detach" button, including when nearby chat alone is left
  there. `nearby` clears the open IMs and shows nearby chat alone: Detach
  must already be available. `nearby-out` also detaches it: its own window
  stays open, the empty Conversations window closes, and "Attach to
  Conversations" reopens it on nearby chat. Opening a private conversation
  afterwards must leave nearby chat detached. With `AURORA_DEMO_CONV_MENU=im`
  without `AURORA_DEMO_CONV_POPOUT`, the friend's menu starts with "Detach".
- **`AURORA_DEMO_CONV_EMOJI=1`**: with `AURORA_DEMO_CONV=aurora`,
  `aurora:compact`, `bubbles` or `bubbles:legacy`, the friend's
  conversation holds two more messages of emoji alone: Nova's three
  (🎉🎶🐺) after her "join me", our 👍 before "On my way!". What to see: in
  the Aurora, Compact and Bubbles styles their pictures about 40 points
  tall (2.4 times the text, sharp: the font's own bitmap), the messages
  that mix text and emoji at the text's size; in the Legacy lines every
  message at the text's size. Capture at frame 900.
- **`AURORA_DEMO_CONV_PREVIEWS=1|states|net`**: with `AURORA_DEMO_CONV`,
  links with their cards (`link_preview.rs`, `ui/chat/previews.rs`). `1`,
  in the friend's conversation: a picture of an unknown site (its card asks
  "Show": nothing loaded before), an animated GIF (its "GIF" mark, the frames
  turning), a picture of a trusted site (hovered: open, copy, hide; right
  click: also hide the site's previews) and our YouTube link (picture, play
  button, title and channel); with `aurora:group`, Loup's picture in the
  group, blurred until clicked. The demo makes these pictures, no network.
  `states`: a picture loading forever (its empty card), an unavailable one
  (the link alone) and a 24 MB one (its size and the limit, open button).
  `net`: the real sites, network needed: two Wikimedia files (an unknown
  site: "Show" loads them, the GIF animated) and a YouTube video, whose
  title and picture come from YouTube. `videos` (#848): Nova's 4:3 clip
  ("Me at the zoo") and our upright short, each card in its video's shape
  (the short as tall as a landscape card is wide, its card no narrower
  than 200 points; the thumbnails without their black bands). `services`
  (#923): links to the pages of image services, read from the saved answers
  of `link_preview/fixtures/` (made-up ids, no network): a removed Lightshot
  screenshot and a Reddit text post (the links alone, no card), then a
  Gyazo page, a Flickr short link, an Imgur album, a Lightshot screenshot,
  a Reddit post, a Bluesky post, an X post and a Bluesky post labelled
  `nudity` (#1051), each with its picture (the demo draws it); the Reddit
  one and the labelled one blurred until clicked; last, two shares of a
  made-up Nextcloud, `cloud.example.org`, that the demo adds to the picture
  sites (#1063, not to the settings): a folder (its card asks to show, then
  finds no picture) and a picture (its card asks to show, as the site is
  not trusted, then shows it). `services:gyazo,flickr`
  keeps the listed services only (`gyazo`, `flickr`, `imgur`, `lightshot`,
  `reddit`, `bluesky`, `x`, `nextcloud`, `none` for the two links without a
  card).
  `clips` (#983): an `.mp4` of an unknown site (the link alone: clips play
  from trusted sites only), a GIPHY WebP animated like a GIF, a Gyazo video
  capture (the thumbnail of the saved answer, a video card with the play
  button, the site as its channel) and our Imgur `.gifv` link (a video card
  with the film icon, no thumbnail). Playing a clip needs the network and a
  real file: send a Gyazo video, Imgur `.gifv` or `.mp4` link of a trusted
  site in a demo conversation. The Legacy lines (`bubbles:legacy`)
  show the same cards, including animated GIFs and consent prompts (#976);
  combine with `AURORA_DEMO_CONV_POPOUT=1` to check independent windows.
  Transcript history and the chat lines at the bottom stay text-only. Capture at frame 900
  (1500 with `net`).
  Ready pictures, GIFs and videos replace their raw URL in all four conversation
  styles (#933). Hover a card: Copy link copies its original URL, Open in browser
  uses the ordinary link-opening path; both tooltips show the URL. Copy image,
  enlargement and detaching remain separate. Captions and labelled links remain
  visible. Hidden, blurred, loading, failed and disabled previews keep their link.
  `AURORA_DEMO_CONV_PREVIEWS=links` shows URL-only GIF/video messages and an image
  with a caption; `links:gif`, `links:image` or `links:video` keeps one of them.
- **`AURORA_DEMO_PREVIEW_ZOOM=1|gif|zoom|pan|window|copy|links`**: with `AURORA_DEMO_CONV`
  and `AURORA_DEMO_CONV_PREVIEWS=1`, enlarge the first loaded still picture
  (`1`), or animated picture (`gif`); `zoom` starts at twice its fitted size,
  `pan` also moves it. The chat body dims, leaving its title bar reachable.
  Test the wheel over the image (zoom around the pointer), left drag,
  double-click or Fit to reset, the close button, a click on the dim margin,
  Escape and Ctrl+W. The chat must not scroll, send text or switch tabs under
  the image; closing restores its editor focus. Resize and move the floater,
  cover it with another window, minimize and reopen it. In a group the first
  click still reveals the blurred card; the next click enlarges it. GIFs
  keep animating. Browser and Copy remain explicit actions. Capture at frame
  900. No network in these four scenarios; normal still pictures request
  detail up to 2048 pixels on opening, with the existing 8 MB download limit;
  on failure their cached preview remains available. GIFs keep their card's
  frame resolution and memory budget.
  `links` hovers the URL actions of a visible card after frame 620, without
  clicking or starting playback; capture at frame 900 to see the URL tooltip.
  `window` opens the independent Image preview floater: move/resize it,
  zoom/drag/reset, then close or minimize the chat; the image stays usable.
  The thumbnail's hover actions are Copy link, Open in browser, detach image,
  Copy image and Hide; enlargement and detached previews also expose both URL actions.
  `copy` scrolls to a loaded thumbnail as needed and clicks its actual Copy
  button after frame 620; it does not open either enlargement. On Windows,
  paste into an Explorer folder: a PNG is copied there, with transparency;
  paste into an image editor: pixels remain available too. Source PNGs stay
  in the viewer cache for later pastes, including after the image/viewer closes.
  A GIF copies its first frame as a still PNG. The tooltip reports preparation,
  success or failure. The copy demo replaces the system clipboard with its image.
- **`AURORA_DEMO_PREVIEW_PLAY=card|window`**: with
  `AURORA_DEMO_CONV_PREVIEWS`, the first video card drawn plays by itself,
  in its card (wider, stop and window buttons under it) or in the player
  window ("Me at the zoo" in its title, open in the browser in its bar).
  With `net` the video really plays (network needed; capture at frame
  1500): YouTube's player inside a page served on 127.0.0.1, with
  YouTube's controls; without that page it shows its error 153. The card
  and the window keep the video's shape (#848): dragged by an edge or a
  corner, the window's picture keeps it at every frame (#941; see
  `AURORA_DEMO_RESIZE`). In its card the cog, window
  and stop buttons show only while the pointer is over the card; the cog
  (also in the window's title bar) opens Preferences › Chat on "YouTube
  videos". Moving YouTube's own volume slider logs `video volume kept at
  N %`, and the next video starts at that level.
- **`AURORA_DEMO_VIDEO_VOLUME=<0-100>`**: demo only, the level YouTube's
  player is set to when a video starts (what Preferences › Chat › Video
  volume keeps). With `AURORA_DEMO_PREVIEW_PLAY=window` and `0`, the
  player's volume icon shows one wave instead of two at frame 1500; no
  `video volume kept` line follows (the player's own 100 % before it is
  set is not taken for a change).
- **`AURORA_DEMO_GIFS=1|klipy|sent|menu|nokey|net`** (#932): the
  Conversations window open on the friend's conversation (or the one of
  `AURORA_DEMO_CONV`), its field's GIF picker open (`gif_picker.rs`,
  `gif_search.rs`). Outside these scenarios the demo has no GIF button,
  as a profile without a key. `1` / `open`: GIPHY's tab on its trending, 24 tiles of
  dots going round drawn by the demo (no network, no key), the categories
  row, "Powered by GIPHY" under the grid; scrolling to the end asks the
  second page. `klipy`: KLIPY's tab, "dance" searched (the field's
  placeholder reads "Search KLIPY"). `sent`: the first GIF is sent once its
  tile shows: its link becomes our message, and its card animates under it
  (with the previews of `AURORA_DEMO_CONV_PREVIEWS`' demo, made here).
  Searching `none` shows the empty grid, the popup the same size. `menu`:
  the picker closed, the GIF button's right-click menu open ("Hide the GIF
  button", "GIF settings…"). `nokey`: the real settings without a key: no
  GIF button, only the emoji one. `net`: the real services, with the keys
  entered in Preferences › Chat › GIFs (the button shows once one is
  entered); network needed, and a free test key allows about 100 requests
  an hour. Capture at frame 900 or later.
- **`AURORA_DEMO_CONV_SELECT=all|part`**: the Conversations window open on
  the local chat with a selection made as soon as its lines are drawn: `all`
  as Ctrl+A (every line, out of sight included), `part` from the middle of
  the last line drawn but two to the middle of the last one. What to see: the
  selected text on the theme's selection (violet, the text in its light
  colour), inverted in my own bubbles (light, violet text), nothing outside
  the selected characters; with the classic lines
  (Preferences › Aurora › Conversations) as with the bubbles.
  For #1225, drag across several messages, copy with Ctrl+C, paste into the
  conversation field and send: the resulting message must retain separate
  paragraphs and author names. Repeat in a private IM and the nearby-chat bar,
  with and without emoji, in all four conversation styles. In Aurora/Compact,
  right-click a selected message and choose Copy message: it must copy the same
  selection as Ctrl+C, including partial first/last messages. With no selection,
  Copy message still copies only its target; Quote still quotes that message.
- **`AURORA_DEMO_CHAT_INPUT=1|long`** (#1226): open Conversations with a
  paragraph draft in its field and the nearby-chat bar. `1` shows four lines,
  including an empty line and emoji; `long` exceeds the four-row viewport.
  Combine with `AURORA_DEMO_CONV` to check Aurora, Compact, Bubbles and Legacy,
  private/group IMs and detached conversations. Paste paragraphs and use
  Ctrl+Shift+Enter: preserve line breaks, replace selected text and support
  undo without sending. Enter sends, Ctrl+Enter shouts in nearby chat and
  Shift+Enter whispers only in nearby chat; in private, group and conference
  conversations it leaves the draft, completion and focus unchanged. The
  field wraps and grows to four rows, then scrolls;
  the dock stays in place, while the full Legacy bar reserves its taller
  layout. Conversation windows keep their size. Sending collapses the field.
  Capture at frame 750 or later. This fixture uses no grid or system clipboard.
- **`AURORA_DEMO_EMOJI_CODES=<text>`**: the Conversations window open, its
  input holding `<text>` with the keyboard and the caret at the end
  (`ui/emoji/codes.rs`). With `hi :hea`, the list of suggestions opens
  above the field: the emoji of the codes starting with "hea" (heart first),
  the first one lit; the arrows move the choice, Tab or Enter take it (Enter
  is not sent), Escape closes the list until the letters change. Typing the
  closing `:` of a known code (`:smile:`) turns it into its emoji; a time
  (`12:30:`), a word before the colon or a link stay as typed.
- **`AURORA_DEMO_SPELLING=1|menu|replace`** (`spelling.rs`,
  `ui/spelling.rs`): `1` puts "Helo, I will teleport to the sandbox with
  my frends" in the chat bar and "Teh avatar is waiting at the landng
  point" in the Conversations input (opened): red squiggles under Helo,
  frends, Teh and landng only (the default English dictionary and the
  Second Life glossary, so "sandbox" and "teleport" pass; the
  glossary's words have no affixes, so "teleporting" is marked as in the
  Legacy Viewer). `menu`: the
  chat bar takes the keyboard at frame 700, its caret at the end, and the
  text menu opens on "frends" at frame 706 (capture at frame 780 with
  `AURORA_FPS_LIMIT=60`): up to five suggestions ("friends" among them),
  Add to Dictionary, Add to Ignore, then Cut… Select All. `replace`:
  auto-replace on for the run (not saved), the bar takes the keyboard at
  frame 700 and gets "brb " at 703, "ty " at 705: it shows "be right back
  thank you ". The tab of the settings: `AURORA_DEMO_OPTIONS=chat:spelling`
  (or `chat:auto-replace`). By hand: the word being typed gets its squiggle
  half a second after the last key; a suggestion replaces its word; Add to
  Dictionary (`user_custom.dic`) and Add to Ignore (`user_ignore.dic`, in
  `<config>/dictionaries`) take the squiggles off every field; another main
  dictionary or an extra one checks again at once; Import… takes a
  Hunspell `.dic` (and its `.aff`). Web and e-mail addresses, and words with
  a digit or `_`, are never marked.
- **`AURORA_DEMO_EMOJI_PICKER=1|<text>`**: the Conversations window open on
  local chat with its emoji picker shown (first tab, smileys); any value
  other than `1` is put in the input field, as if picked. With
  `AURORA_DEMO_GLYPHS=1`, the emoji line of the chat is visible beside it;
  with `AURORA_DEMO_CHATCMD=<same text>`, the line is sent.
  What to see: colour emojis in every cell, centred, with their
  proportions (the 2D font's images are wider than tall), and the same
  colour emojis, same size, in the input field and in the sent line.
  `AURORA_EMOJI_FONT=<file>` compares two fonts.
- **`AURORA_DEMO_CRASH=panic|native|missing|normal|recovery|freeze|oom|overflow|thread`**, only
  with `AURORA_DEMO`: triggers a Rust panic, a native Windows
  exception, an abnormal exit without a trace, a normal close or an
  exit without a watcher. The `recovery` mode makes it possible to check the
  recovery at the next launch (relaunch with `normal`). Linux has no
  watcher: every mode's window opens at the next launch, as with
  `recovery`, and a session closed normally is removed then, as is one
  ended by SIGTERM, SIGHUP or SIGINT (`kill -TERM` gives no window). No
  connection and no GPU initialisation, except for `freeze`: the viewer
  starts, its main thread stops answering once the window shows, and 13 s
  later the process ends itself with exit code 1, as Task Manager's End task
  would. The window then says "Frozen, then ended" and `public-report.md`
  holds `| Crash | Freeze |`, `| Window | not answering for the last … s |`,
  the main and render threads' stacks and the "Last state" table. `oom`
  asks for 64 TB at start: the allocator's failure path writes `oom.txt`
  and the process fails fast (`0xC0000409`); the report says
  `| Crash | Out of memory (70368744177664 bytes) |` with the allocation's
  stack. `overflow` recurses until the stack overflows (`0xC00000FD`): the
  report is a native exception with its stack, written by the vectored
  handler. `thread`, once the window shows, makes a thread named
  `aurora-net` panic: the viewer ends the session at its next frame, the
  window says "A part of the viewer stopped working" and the report adds
  `| Thread | aurora-net (the session was ended) |` to the panic. The window must keep the language
  and the theme of the stopped session, allow the trace to be selected and
  copied and open its folder; no window after a normal
  close. The screenshots are taken after three frames of the independent
  window. `AURORA_DEMO_CRASH_DIR` makes it possible to use one temporary
  folder per scenario, without touching the real reports. While a
  viewer runs, its session folder holds a `watching` marker; without it
  the watcher did not start (its error is the first line of the console)
  and no window would follow a crash.
  "Send a crash report" opens the preview of the issue to publish
  (docs/BUG_REPORTING.md); it is absent when Preferences › Privacy says
  Never.
- **`AURORA_DEMO_CRASH_SEND=ask|always|never[,connected][,preview][,authorize][,sent][,fail]`**,
  with `AURORA_DEMO_CRASH`: the crash window with a simulated GitHub (no
  network, no credential store). The first word replaces the Preferences
  choice; `connected` starts with an authorization, `preview` opens on the
  preview, `authorize` also starts the connection (code `DEMO-1234`, then
  authorized after three seconds), `sent` shows a report already sent as
  issue 4242, `fail` makes the creation fail. `always` opens straight on
  the preview and, like every mode, publishes only once a few words are
  written in "What were you doing". A real authorization and a real issue are only tested
  by the human, outside the demo.
- **`AURORA_DEMO_BUG_REPORT=1|details|image|image-ui|direct`**: no effect
  while the built-in bug report form is disabled ("Report a bug" opens
  GitHub's issue chooser, see [BUG_REPORTING.md](BUG_REPORTING.md)); what
  follows describes the form once enabled. Opens the
  report after loading (frame 550). `1` shows the description,
  `details` the information with the location unchecked and the diagnostics
  filtered, `image` captures the scene alone, `image-ui` keeps the interface,
  `direct` selects direct sending. In all these modes, sending stays local and
  opens neither GitHub nor the browser. See [BUG_REPORTING.md](BUG_REPORTING.md).
  Regression modes: `refresh` takes a screenshot then enables the interface
  at frame 700 and disables it at frame 900; `language` switches the open
  form to English at frame 700, then to French at frame 900. Take screenshots
  at frames `650,850,1050` to compare. To test the language choice in
  Preferences, do not pass the `--language` argument.
  Human validation of real sending, without demo mode: authorise GitHub,
  select an image, then send. The issue must hold the reviewed text only,
  and the report's `public` folder must open so the picture can be dragged
  into the issue.
- **`AURORA_DEMO_LOGIN=cards|cards2|cards8|derez|mfa`**: synthetic account carousel with three, two or eight accounts and generated landscape thumbnails; `derez` starts the shortened fragment transition and holds synthetic connection progress inside the selected card, while `mfa` opens the two-factor challenge. No grid connection, saved-scene reads or credential-store access. Other card-mode clicks report that demo mode cannot connect. Cards keep their own grid/start selectors without password controls or a choice heading/count. Add account remains outside the carousel; arrows appear only above three accounts; smaller catalogues remain in a fixed row even in narrow windows. Check 800 × 733 and 1280 × 720 layouts, vertical scrolling at 800 × 560, and the 30-day checkbox in the MFA dialog.

- **`AURORA_DEMO_LOGIN=connecting|first-connecting|add-connecting`**: synthetic waiting state in the selected card, inline first-account form, or Add form. Capture early and late frames (`AURORA_CAPTURE_FRAMES=20|40|90`) to check the brief animation, status/bar placement and persistent Aurora background. No black progress screen and no grid request. Confirm that inputs remain read-only while pending; a real error/MFA/terms challenge must restore the form, and successful authentication must switch to region loading.
- **`AURORA_DEMO_LOGIN=cards1|derez1`**: a single saved account; `derez1` starts its validation. With no neighbour card to shatter, a faint echo of the chosen card breaks apart towards both sides from under it. Capture `AURORA_CAPTURE_FRAMES=30|36|42` and compare with `derez` (three cards).
- **`AURORA_DEMO_LOGIN_WAIT=logout|sudden|refused`** (#956, `login_wait.rs`): the demo account's previous session logged out when the viewer started (`logout`: Log in waits up to 10 s), was never seen ending (`sudden`: a crash or a killed viewer, up to 60 s from its last sign of life), or the offline login server answers once as the grid does while it logs out a previous session (`refused`: the grid's message, then a 60 s countdown before trying again). The login screen shows the countdown in the connection progress (“Previous session… 7 s · Esc cancels”, “Session still open… 57 s” after a refusal); Escape cancels it and gives the form back, the end of the countdown sends the login (the demo then says it cannot connect). Use it with `AURORA_DEMO_LOGIN=cards` or `connecting`, and `AURORA_DEMO_LOGIN_SUBMIT=<frame>` for captures. The log says `login waits <n> s for the previous session (…)`. Nothing is read from or written to `last_sessions.json` in demo mode.
- **`AURORA_DEMO_LOGIN=add-mfa`**: one saved account and the Add form open with a second synthetic identity typed in; Log in sends it to the offline server, which asks for the second factor 1.2 s later. The challenge must stay in the Add form with the new identity (never the saved card's), and the log line `demo login: request for a new account` names which identity was sent, without the name. `AURORA_DEMO_LOGIN_SUBMIT=<frame>` presses Log in at that frame for captures (`AURORA_DEMO_LOGIN_SUBMIT=40`, `AURORA_CAPTURE_FRAMES=170`). The demo never touches the system credential store.

- **`AURORA_DEMO_LOGIN=drag|drag2|drag-card|drag-card2`**: eight/two synthetic profiles with a left-button side-grip or portrait gesture injected into egui at frames 20–44. Capture frame 40 while held and frame 75 after release. With eight profiles, check continuous card motion, clipped neighbours, circular wrapping, final alignment and central selection without connecting. With two profiles, the same gesture must keep the cards fixed, without arrows or connecting. Input injection never moves the OS pointer. Manually drag in both directions from portraits, place images, names, card padding and Connect buttons; release over a card/outside the window and resize during motion. A simple click still connects, while grid/start menus and text-field drag selection remain usable. Also check Enter, dialogs and short-window vertical scrolling outside the browse row.

- **`AURORA_DEMO_LOGIN=first|first-mfa|add|password`**: full inline form with no saved accounts, its MFA challenge, the full Add dialog over two existing profiles, or the compact password prompt for an existing profile. Check the frosted glass of Add, password and MFA dialogs against the login cards, including opaque skins and compact/large windows. Check username/password/remember controls on the full form, read-only account identity and no grid selectors in the compact prompt, and the 30-day/one-use MFA choice. Submitting a demo form never registers a profile or connects a grid.

- **`AURORA_DEMO_LOGIN=manage|manage8|manage-confirm`**: open the redesigned account manager with two/eight synthetic accounts or a pending account-removal confirmation. Check frosted Aurora dialog surfaces, portrait rows, password/MFA summaries, account/grid/start search, scrollable results, and separate Forget password / Forget 2FA / Remove actions. Confirmations identify the account even when the catalogue is refreshed or reordered. Actions in these scenarios change only the in-memory fixture. `AURORA_DEMO_LOGIN=manage-drag` injects a row drag at frames 20–40: capture frame 34 for the floating preview and insertion marker, then frame 60 for the new order. Manually drag from the grip, portrait, name, padding and action-button area; simple action clicks must keep their confirmations. Check moving upward/downward, filtered rows, edge scrolling, Escape and releases outside the list/window. Reordering must preserve the selected identity in the running viewer. Reopen the manager and restart the viewer to verify saved order, remembered credentials and grid/start/last-place metadata; also test a metadata edit/new account in another viewer before reordering.

Left/Right select the preceding/following account. Above three profiles, keyboard and button arrows centre the selected card and wrap around; the central card carries the selected effect by default. With three or fewer profiles, keyboard selection preserves every card position. Enter connects the selected account. Arrow keys in a text field keep their editing behavior, and open menus/dialogs keep their own keyboard handling. Clicking a card or its Connect button connects directly; opening its grid/start selector does not. Enter also submits a valid password/MFA prompt. Mandatory updates block every connection path.

`AURORA_DEMO_LOGIN=hover|wheel-gap|arrows|static-arrows` injects synthetic hover, a wheel notch over a card gap, or keyboard selection with eight/three accounts. Capture frames 18 and 40 to compare fixed positions, hover lighting and centred selection. Wheel input over cards, gaps and previews must browse exactly once per notch; small trackpad deltas accumulate. Menus, dialogs and focused text fields retain input priority, and browsing never connects.

Cursor checks: hovering portraits, place images, names, card padding or the browse area keeps the normal pointer; fields and buttons retain their usual cursors. A grasping hand appears only while the left-button drag is active, then clears immediately on release even during inertia. Windows uses the existing Tabler native bitmap to avoid the system Grab/Grabbing icons appearing as directional arrows. Offline `drag-card` exercises this path; automated screenshots do not include the OS cursor, so its appearance needs a human check.

Responsive login checks: compare `cards2` at 1600 × 900 and 2560 × 1440. Portraits, scene previews and cards grow moderately (at most 22%), Manage/Add remain close below the cards, and Version/News stay visible on the right. Check `cards|cards8` for at most three complete central cards, faded previews only when additional accounts exist, and arrows outside the previews. Compare `drag-card` at frames 40, 48 and 90 for pointer tracking, longer coasting and final alignment. Simple clicks and Enter must still connect; clicking a preview must reveal the full account card before its validation animation. At 800 × 733 and the scrollable 800 × 560 minimum, check reduced central capacity, readable controls, edge clipping, community links, footer version and News dialog; first-account MFA and credential dialogs remain accessible. The subtitle and footer information are also shared with `remembered|empty`.

Human grid validation: verify that the first account appears only after successful login; add another account, leave its password unremembered, and check the compact prompt on its next connection. Alternate accounts and grids with different MFA remember choices. Visit different places, log out, and confirm that each card keeps its own scene image/region across restarts and simultaneous viewers.

Human grid validation: remember MFA for account A and B, restart the viewer, then log in with A and B again. Neither should invalidate the other's authorization. An unchecked 30-day choice must prompt for a fresh code at the next login for that account only. Also test the same user on different grids and two simultaneous viewer instances. In the manager, forget only one account's password or MFA authorization, and verify the other's retained credentials. Expiration and account isolation are covered by synthetic protected-store tests; the agent never connects to a grid.

- **`AURORA_DEMO_LOGIN=remembered|empty`**: offline login screen
  with a synthetic marker of a saved password, or an empty password
  field (no access to a grid or to the credential vault).
  The news there is the one built into the viewer (`news/<language>.md`),
  in the language of `--language`.
- **`AURORA_NEWS_URL=<folder url>`**, outside demo: the viewer downloads
  `<url>/<language>.md` instead of the `news/` folder of `main`. Two forms are
  accepted, any other is ignored with a warning in the log:
  a server on this machine (`http://127.0.0.1:<port>/`, `localhost` or
  `[::1]`), for example `python -m http.server 8123 --bind 127.0.0.1` run
  in the `news/` folder of the worktree; or another branch of the repository
  (`https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/<branch>/news/`),
  to review a news item before merging its PR. What is downloaded
  this way is not kept in the cache. The panel first shows the news
  already known, then that of the file as soon as it is received; changing
  the language in the preferences reloads the news of that language. If the
  download fails (offline, missing file, file with no news item), the log
  says `Login news not updated` and the panel does not change: last
  download (`news-<language>.json` in the cache folder) or, failing that,
  the files built into the viewer. The demo and the update test
  installation never download the news, even with this
  variable.
- **`AURORA_DEMO_UPDATE=1|progress|ready|ignored|beta`**, with `AURORA_DEMO=1`: offer
  of a synthetic version, download at 42% or download finished; `beta`
  offers a nightly beta (`0.7.7-beta.20261025.1`), as a viewer on the Beta
  channel sees it.
  "Update" animates a simulated progress; "Cancel" goes back to the
  offer. No download and no file replacement. Login
  is inert and the login preferences stay intact.
  After "Skip this version" or "Later", the "Check for
  updates" button of the Version panel reopens the offer. The `ignored` mode
  starts directly with the dialog closed to try this flow.
  In a small window, the button stays reachable at the top right.
  `AURORA_DEMO_UPDATE_MAINTENANCE=cache|settings|both` shows the forced effects;
  when absent, the dialog announces the usual preservation and migrations.
- **`AURORA_DEMO_GPU_DRIVER=old|old-linux|aged`**, with `AURORA_DEMO=1`: the
  driver check (`gpu_driver.rs`) reads a made-up card instead of the real one:
  `old` a GeForce GTX 1060 on NVIDIA 546.33 (under the 572.16 minimum),
  `old-linux` the same card on the Linux driver 565.57.01 (under the Linux
  minimum, 570.86), `aged` an AMD card whose driver was installed three years
  ago. The login screen shows the caution-edged card in its top left corner
  (with `AURORA_DEMO_LOGIN=cards` or `empty`); "Get the … driver" opens the
  vendor's page, "Ignore for this driver" hides it and saves
  `gpu_driver_dismissed` in the demo profile (clear it there to see the card
  again). Without the variable the real driver is checked: the log line
  `graphics driver:` says the verdict.
- **`AURORA_DEMO_GPU_STALL=<frame>[:<seconds>]`**, with `AURORA_DEMO=1`: the
  render thread holds that frame (30 s by default) as a graphics card that
  stopped answering would. After 2 s the log says frames wait; the window
  keeps answering (no "Not responding", sound and chat go on, the image
  stays as it was). After 6 s the native box "the graphics card is not
  responding" shows; Yes keeps waiting (shown again a minute later if still
  stuck), No closes the viewer in at most a few seconds, logged out, and the
  crash session holds `gpu-stall.txt` (on Windows with the render thread's
  stack, and the report says `| Crash | GPU not responding |`; elsewhere
  the stall's length alone), and the next start offers the safe graphics.
  When the frame ends the box goes away by itself and drawing resumes
  (`the graphics card answered again`).
  With `AURORA_FPS_LIMIT=60`, frame 600 comes about 10 s after the start.
  The window's close button during the stall closes at once (the exit
  prompt cannot be drawn).
- **`AURORA_DEMO_GPU_LOST=<frame>`**, with `AURORA_DEMO=1`: the GPU device is
  taken as lost at that frame, as the device-lost callback does after a
  driver reset (the log says `GPU device lost (Unknown): AURORA_DEMO_GPU_LOST
  at frame …`). No frame is drawn any more (the image stays as it was) and
  nothing waits on the GPU: a photo asked then fails instead of ending the
  session with a Rust panic (#1237). At once the native box "the graphics
  card was lost" shows, with what to try; its OK closes the viewer, logged
  out, and the crash session holds `gpu-lost.txt`: the report says
  `| Crash | GPU device lost |` and `| GPU device lost | yes … |`, and the
  next start offers the safe graphics. The window's close button closes at
  once too. A real loss cannot be caused here: the flag is set as the
  callback would set it, wgpu's own device stays alive.
- **`AURORA_DEMO_OVERLAY_HOOK=<file>`**, with `AURORA_DEMO=1` (Windows): the
  overlay hook watch (`overlay_hooks.rs`) counts that module file name as
  loaded, for example `DiscordHook64.dll`, `GameOverlayRenderer64.dll`,
  `RTSSHooks64.dll` or `graphics-hook64.dll`. A known one gets its warning
  in the log (`overlay hook loaded in the viewer: …`) and its line in
  `overlay-hooks.txt` of the crash session; any other name does nothing.
  With `AURORA_DEMO_CRASH=freeze`, the report then says
  `| Overlay hooks | Discord (in-game overlay or Go Live capture), DiscordHook64.dll |`
  (#1342). Nothing is injected: the image is not touched.
- **`AURORA_DEMO_SAFE_GRAPHICS=offer|active`**, with `AURORA_DEMO=1` and
  `AURORA_DEMO_LOGIN=cards` or `empty`: the login screen's safe graphics
  card, above the driver warning. `offer` is what a start after a GPU stall,
  a device loss or a frozen window shows ("Use the safe graphics" applies the
  Potato level and keeps the level's values in `graphics-before-safe.json`
  of the profile folder; "Keep my settings" hides it); `active` is what
  shows while those values are kept ("Restore my graphics" puts them back,
  "Keep the safe graphics" drops them). Without the variable the real state
  is shown: the crash root's `gpu-trouble` mark, consumed at start, or the
  file kept aside.
- **Isolated real installation**: `scripts/test-update.ps1` prepares packages
  0.2.100 and 0.2.101 in `.scratch`, serves the second on loopback and
  launches the first.
  The server slows the ZIP down to test Cancel. Build in the ordinary
  `target` of the worktree, dev profile, then restoration of the normal binary
  at its Git version; no tag or release published. `AURORA_UPDATE_TEST_ROOT` is
  only accepted for an exe in its `install` subfolder, with an
  `aurora-update-test.json` marker defining a loopback HTTP feed. Settings,
  cache and logs are isolated under this root; login and credential vault inert.
  `AURORA_UPDATE_TEST_VERSION` is reserved for these binaries and refused in
  release or with `AURORA_VERSION_TAG`. The synthetic binaries refuse any normal
  launch. Test Skip → Check, Update → Cancel, then Install and
  restart: the title must go from 0.2.100 to 0.2.101. Close the viewer and stop
  the displayed server PID before cleaning up. The title, language and
  FPS limit arguments given to the script are kept after the restart.
  `-Beta` serves `0.2.101-beta.20261025.1` as a GitHub pre-release and starts
  the isolated profile on the Beta channel: the offer shows; Preferences ›
  Language and updates › Versions › Stable hides it, Beta brings it back.
  The script needs PowerShell 7 (`pwsh`).
  Add `-ClearCache` and/or `-ResetSettings` to the script to test the
  publishing choices. The dialog must announce the effects before download
  and restart. After the new viewer is closed, `cache/update-marker.txt`
  must be gone if the cache was to be cleared; `profile/settings.json`
  must be back to the default values if requested (initial test distance:
  300 m), and `profile/update-marker.txt` must stay present in all cases.
  Add `-Required` to impose the next version: no Skip or
  Later choice, Esc does not close the dialog, and the login button stays
  disabled with the blocking reason even if the fields are filled. In
  any isolated installation, a label indicates that login to a grid
  is disabled; only enter synthetic values for this test.
  `AURORA_UPDATE_ACK` and `AURORA_UPDATE_ROLLBACK` are internal signals between
  processes, not scenarios to set by hand.
- **`AURORA_DEMO_CHAT_LINES=1|legacy|volumes`**: nearby chat said at the arrival,
  shown above the chat bar with the Conversations window closed: two lines
  of one resident, an emote, an object's line, a long line with a link, a
  shout (only the 6 most recent lines show, as always). `1`: Aurora's cards
  (Preferences › Aurora › Nearby chat lines), one per sender in a row, the
  picture, the name, the time if shown, SHOUTS / WHISPERS / IM after the
  name, the lines under it, each fading on its own; `legacy`: the Legacy
  Viewer's one-line look for the session (nothing saved). Check them over a
  dark and a bright world (`AURORA_DEMO_SKY=2,4,2`): readable on both, the
  name, the picture and the links clickable, the world clickable around.
  `volumes` shows the same normal, whispered and shouted messages from us
  and an object in both the cards and Conversations, visible together to
  compare their colours, whisper italics and shout weight. These demo cards
  remain visible for an hour without changing the saved timeout.

- **`AURORA_DEMO_OBJECT_CHAT=aurora|compact|bubbles|legacy|lines[:on][:im][:imlines]`**
  (#475, #741): nearby chat said at the arrival, two residents among a
  greeter, a tip jar, our HUD's owner-say, a door's emote and a vendor's
  llRegionSayTo with a link, then two notices (a friend online, a
  teleport). The Conversations window shows the nearby chat
  in the style named (`1` is `aurora`), or `lines` the cards above the chat
  bar with the window closed. `:on` turns Preferences › Chat › "Compact
  messages that do not come from a person" on for the run (nothing saved):
  each object's line on one small row (cube, name, muted text), the notices
  with the icon of their kind; the nearby chat's unread count counts the
  two residents' lines only. Compare a run without `:on`: the same lines in
  the style's usual look. Hover a cube: the object's name; click it or the
  name: the object's inspector (its menu: inspect, ignore). `:im` shows the
  friend's conversation instead, with a call's and an automatic reply's
  lines: unchanged with `:on` alone, compact with their icons (a phone, the
  "i") with `:on:imlines` (the sub-option "Compact the conversations'
  system lines too", off by default). Above the chat bar, the notices
  carry their topic's icon too. After the HUD's owner-say comes its script
  error (#1263): in red under a red warning sign, in a group of its own, in
  every style and in the compact rows; the Script debug window does not
  open, yet lists it. The tip jar's script error is not ours: shown nowhere.
  With Preferences › Chat › Script errors › "Show script errors in the
  nearby chat" off, the error leaves the chat and the window opens.
  `AURORA_DEMO_OPTIONS=chat:script-errors` opens that section: the switch
  on, "Also say them on a chat channel" off, its channel (1) among its
  sub-settings.
- **`AURORA_DEMO_CONV=bubbles[:busy|group|local|legacy]`**: the Conversations
  window as a messenger, placed at 700 x 560 px. Without a suffix, the
  conversation with an online friend over two days: their bubbles on the
  left with their picture and a green dot under the last one of each
  group, mine on the right on the violet to indigo gradient, the time under
  each group, "Yesterday" above the first message and "Today" between the
  two days, the corner under the sender square on the last bubble of a
  group; round pictures; the head shows the picture, its dot, the display
  name and the user name. The friend is typing (also with `legacy`): their
  tab reads "<name> is typing…". `busy`: a resident in the
  region playing the do not disturb animation, after their automatic
  response: red dot and "Do not disturb" pill in the head, red dot in the
  list. `group`: the demo group chat, the sender's name opening each group
  of bubbles. `local`: the nearby chat (residents, an object with its box
  and its name in its color above its group, the viewer's notices in
  centered pills). `legacy`: the friend's conversation in the legacy lines
  of the setting `chat_style` (`Legacy`; otherwise `Bubbles`), which the
  demo does not save.
- **`AURORA_DEMO_TYPING=1`**: our typing in the nearby chat without a key,
  held from our arrival on (`nearby chat: typing started` in the log). The
  demo plays its own stand-in for the typing animation (a grid asset it
  cannot download): forearms forward at the waist, wrists tapping in turn;
  it also plays on the crowd's typing avatar (`AURORA_DEMO_TAGS=1`, Lune;
  Iris types without it). Our plate shows the typing dots beside its microphone
  (Preferences › Chat › "Show typing in name tags"); seen from behind, the
  forearms are hidden in front of the body. Typing by hand in the chat bar
  does the same, and stops when the line is sent, the field emptied or left,
  or after five seconds without a key.
- **`AURORA_DEMO_CONV=aurora[:flag…]`**: the Conversations window in
  Aurora's workspace (the default style), alone on screen (People and the
  inventory closed), 960 x 560 px. The column holds two groups: Northern
  Wolves shows its insignia, Aurora Builders (none) the group glyph. Flags, joined by `:`: `im` (default:
  the friend's conversation over two days, the last three lines unread
  under the red "New" line, the friend typing), `busy` (Tess in do not
  disturb), `group` (Aurora Builders with its members' panel: MOD tag,
  an amber bar on the message that names us, a place link), `local` (the
  nearby chat: object tag and box, shout and whisper tags, our emote in
  violet italics, notices with an icon, someone typing, the volume's
  menu, Whisper / Say / Shout, at the left of the field; the channel chip
  is hidden as by default); `compact` for the Compact style (one row
  per message, time, name and tags); `medium` (700 px), `narrow` (460
  px: the column becomes a rail of pictures) or a width in points;
  `call-incoming` (Tess calls: the banner and the pulsing entry),
  `call` (the group's call: the stage, speaking ring, Tess muted by a
  moderator, the speaker in the column), `call-mini` (the stage reduced to
  its strip), `call-ongoing` (Nova's group call left ringing: the strip
  with Join, the green call button), `call-away` (in the group's call
  with the nearby chat shown: the column's call panel), `call-incoming-group`
  (Nova's group call ringing: Join / Ignore), `call-p2p` (our call to Tess),
  `conference` (a voice conference with Tess and Nova), `new` (the menu of
  the "new conversation" button beside the column's search field open:
  Private message, Group chat, Conference), `volume` (with `local`: the
  volume's menu open, above the field; with `narrow` the button keeps
  its icon), `channel` (the "/0" channel chip shown, as Preferences ›
  Chat › Show the channel selector does), `friend-away` / `friend-offline`
  (the friend of `im` away in the region or offline: the selected entry's
  dot keeps its crescent or hollow ring, cut in the entry's violet face;
  with `narrow` too), `sets` (the contact sets Family, with the friend and
  Loup, and Clubbing, with Loup and Tess, both shown apart, Family the
  higher: the column opens with a FAMILY section holding the friend and
  Loup after Family's heart symbol, then CLUBBING with Tess after its
  colour's dot, above Private messages; #1104). The calls are those of
  `AURORA_DEMO_CALL` and start 3 s after the login: capture at frame 780
  with `AURORA_FPS_LIMIT=60`. What to look for: the layout of the
  validated mockups (variant A, and B with `compact`); the column's entries, a message's right-click menu (copy, quote, its author's menu; no buttons and the arrow under the pointer) and the head's buttons all act
  (the other `AURORA_DEMO_CONV` scenarios show the window in the saved
  style, Aurora by default).
  Sent-message recall: send two messages, then type an unfinished draft.
  Ctrl+Up walks back through sent messages; Ctrl+Down walks forward and
  restores the draft. Repeat in a private IM, a group and a conference:
  each has its own list. With `aurora:local`, send from the bottom chat bar
  and recall in the nearby conversation, then send from the conversation
  and recall in the bar. The entries are shared; their drafts and browsing
  remain independent. Check all four chat styles, repeated Ctrl+Up at the
  oldest entry, consecutive duplicate sends, channel prefixes and emoji.
  No count limit or disk persistence; closing an IM/group/conference
  discards its recall, while hiding Conversations keeps it.
  Chat context menus (#557): in an active group conversation, right click
  a resident's name or picture, or a message in Aurora/Compact. Incoming
  messages must leave the menu and its original target in place, so Ignore,
  Copy and Quote can still be used. Repeat after scrolling up and in all
  four styles. New messages must appear immediately with the menu open,
  following the live end or retaining the normal manual-scroll position.
  Escape, a click outside or choosing an action closes it normally. Moving or
  resizing any window must dismiss open context menus immediately (#589), even when
  the menu belongs to another window. Check all edges/corners, the mini-map,
  and the world's list and pie menus; hovering a grip must keep the menu open.
  Moving the title bar or background must close the menu during the drag,
  before release; small click jitter must leave both the window and menu still.
  Moving or resizing the main viewer window must dismiss menus too. The
  original target must remain available when the 2,000-message live buffer
  evicts its line; only the menu's original target is retained.
  Check that following the live end, manual scrolling, find and text selection still
  work afterwards. The offline regression injects messages between actual
  pointer frames: `cargo test --workspace incoming_messages_keep_chat_context_menus_open`.
- **`AURORA_DEMO_CONV=1|names[:narrow|wide]|many|tp[:sent|reject]`**: `1` opens the conversation of
  the demo group with its participants. `names` shows names longer than
  their space (issue #124): two residents (long display name with user
  name, wide and accented letters) and a conference with a long title
  in the Conversations list, with unread messages, and these residents
  in People › Near you. Both windows are placed at a known
  size, whatever the saved layout: 600 and 330 px wide,
  400 and 230 with `:narrow`, 900 and 520 with `:wide`. What to see: each name
  cut by "…" before the unread badge, the time of the message or the
  distance, without covering anything; the windows keep the requested width;
  the whole name is shown when hovering a cut name (for a screenshot:
  `AURORA_DEMO_POINTER="85,633,s"` on the conference, `"120,377,s"` on a
  nearby resident, 1600 × 900 window). `many` gives 24 conversations to a
  Conversations window of 600 × 320 px and opens the last one: the window
  keeps its size, Contacts and Nearby chat stay at the top of the list, the
  conversations scroll under them (wheel, or the bar that shows at the
  right edge of the list under the pointer) and the list is scrolled to the
  conversation that is open. `tp` (issue #280): a message from Tess Touch
  opens her conversation, then the avatar menu's "Offer teleport" asks for
  the invitation: "Join me in Aurora Demo" (or the French text) to edit,
  Send / Cancel. `tp:sent`: that invitation is sent to her and to a resident
  who has no conversation. What to see: "Teleport offer sent to Tess …" (a
  link to her name) at the end of her conversation, without an unread badge;
  no new conversation for the other resident and nothing in the nearby chat
  (that notice only goes to the resident's transcript, when instant messages
  are logged). `tp:reject`: "Request teleport" while teleports are rejected
  only shows the warning, with OK and no message to type.
- **`AURORA_DEMO_MAP=search:<text>`**: the world map with the search
  already typed (`search:o` lists several demo regions). What to see: in the
  full-frame layout (the default), the suggestions grouped under the search:
  Regions (rating badge, people), Landmarks, Online friends, History; ↑ / ↓
  move the highlight, Enter picks, Esc clears. With `legacy` the side panel's
  results, each with the navigation bar's rating badge (G / M / A) before its
  name; a region offline or unknown keeps its state in text.
- **`AURORA_DEMO_MAP=` words separated by commas** (full-frame layout):
  `drawer:landmarks|history|friends` opens the drawer under the search on
  that tab; `landmarks` adds 204 landmarks in four folders (Clubs, Beaches,
  Walks, Markets) spread over the demo regions and opens the drawer: the
  folders fold, the search filters them, each one is a pink pin on the map
  (layer "Landmarks"); zooming and panning must stay smooth (AURORA_PROFILE=1:
  the `maps` part). `friend` follows Nova: the card shows "Loading…" then her
  region, the drawer's Friends tab lights her line; Orion, who does not share
  his position, is greyed. `legacy` shows the side panel layout without
  touching the setting (Preferences › Aurora › World map). What to see: the
  place card at the bottom left (rating, people, X / Y / Z, distance, five
  buttons on one line), the layer pills at the top right (greyed when zoomed
  out), zoom / my position / home at the bottom right; the panels fade while
  the map is dragged and never let a click through to the map.
  `beacon` shows a nearby destination without the world-map window: the
  bright upper column stays visible up to the screen edge, with a pin and a
  readable location / distance label. A second column descends from the
  destination to altitude zero, as in Firestorm; with the default skin it
  is mauve (`violet_light`), while the upper column is red. Both use the same
  glow and viewport clipping. The pin has no extra circle; its information
  card shows the frosted scene under the theme's tint, border and shadow,
  including beside the off-screen arrow. Opaque skins keep an opaque card.
  Hover the pin or its label for
  "Click to remove beacon"; click either to remove it from the scene
  and mini-map. `beacon-offscreen` puts the destination behind the camera:
  a clickable direction arrow and its label stay within the screen.
  `beacon-click` presses the pin at frame 700 through the normal mouse
  input path; the log must say `cleared=true`, `mouse_mode=None`,
  `pending_pick=false`. Capture frames 650 and 780 for before / after.
  With `legacy`, "Remove beacon" is in the position actions, at the former
  "Clear" button's location. Its tooltip explains removal from all three
  views. The full-frame card's removal tooltip uses the same name and
  explanation. `coordinates,legacy` opens the side panel without a beacon:
  X / Y / Z show our current position and remain editable. Commit a new
  value with Enter or
  by leaving the field: a destination appears without teleporting. Remove
  it: the coordinates return to our position and can be edited again.
- **`AURORA_DEMO_APPLINKS=1|<command>`**: Tess Touch's conversation and the
  chat console show one `secondlife:///app/...` link of each command: a
  group (its name), a link written with its own text, the "About Land"
  parcel (its name once the demo server answers), an object (`objectim`),
  an inventory item, two experiences (the region's treasure hunt by its
  name; the experience of `AURORA_DEMO_EXPERIENCE`, "unknown" without that
  scenario), a classified and an event (their address, as in the Legacy
  Viewer), the map, a chat on channel 42, a search, a resident's IM, the
  same resident's profile glued to the sentence ("…/about's hat": the
  name, then "'s hat" as text), a folder to wear and an unknown command. Each opens what the Legacy Viewer
  opens on a click (group profile, place profile, remote object
  inspector, inventory, experience profile, the classified or the event
  alone in Search, the map); the folder to wear is refused (a notice), as
  is the unknown command. With a command name instead of `1` (`group`,
  `parcel`, `objectim`, `inventory`, `experience`, `classified`, `event`,
  `worldmap`, `chat`, `search`, `agent`, `wear_folder`, `frobnicate`) its
  first link is clicked once the scene is set, for a capture. Also accepts
  `profile`, `pay`, `report` and `inventory-show`. Append
  `:restore` to a window command: minimize its destination at frame 620,
  repeat its action through the shared UI dispatch at 660, and log its
  minimized state and height at 760. Capture `640,780` to compare the bar
  with the restored window. Plain `parcel:restore` uses Places; the unit
  regression also exercises standalone place details and both profile looks.
  `actions` shows identical avatar labels linking to profile, IM, payment,
  teleport offer, friendship and report actions, followed by other native
  targets, plus bare profile/IM/payment/group/unknown URLs inside sentences
  as when pasting into the chat input. Their text must share the surrounding
  words' baseline; the unknown command shows a localized "Unsupported link"
  fallback, keeping its original URI in the context menu. Compare their
  persistent action icons in the conversation
  and local chat overlays without hovering, in English/French and each chat
  style. There must be no added action words before the original link text.
  Custom link text must not hide the action; copying a transcript
  must retain the message content without its visual markers.
  `preview` sends one short message with a resident, a group and a place
  link from Tess Touch and the welcome sign, and leaves Conversations on
  the nearby chat (#947): the column's entries (the nearby chat, Tess)
  and the message's card show the resident's and the group's names and
  "Teleport to Aurora Demo (128,128,25)", never a `secondlife:///app`
  address. With `AURORA_DEMO_NOTIFS=1`, Nova's message at frame 700
  carries the same links: its card shows them by name (capture around
  frame 760).
- **`AURORA_DEMO_SLURLS=1|offer`**: Tess Touch's conversation, the chat
  console and the notification list show the same place links, one per
  SLURL form: the plaza (General), the lagoon (Moderate), Nordheim (Adult),
  the pine grove and Atlantide. What to see: G, M and A badges before the
  first three at once; the pine grove gets its G badge about 3 seconds
  later, when the demo server answers its lookup; Atlantide does not exist
  and stays without a badge. `offer`: the "Offer teleport" dialog for Tess
  shows our region with its badge. `AURORA_DEMO_PLACES=history` shows the
  badges on the teleport history (Ahern, unknown to the demo, has none);
  `AURORA_DEMO_MAP=1` shows one next to the tracked place's label.
- **`AURORA_DEMO_MATURITY=agree|disagree|fail`**: the Preferences open on
  Interface › Content and the demo account (adult, every rating) asks for
  "General". `agree`: the server accepts, the list shows "General".
  `disagree`: the server keeps "Adult"; the request is sent three more times
  (the log shows each retry), then the list goes back to "General, Moderate,
  Adult" and the notification list shows "Content rating not changed".
  `fail`: the request fails, the same notice at once. A teleport asked
  meanwhile waits for the answer and is dropped on a failure.
- **`AURORA_DEMO_HOME=ok|refused|region|fail|udp|forbidden[:set]`**: the
  demo region answers World › Set Home to Here (also the right click of the
  home button). `ok` (the default): the capability keeps the place in whole
  metres, then "Home position set." (a notification and a nearby chat line;
  the log shows `home: set in region`). `refused`: not our land
  (ListValidHomeLocations); `region`: CantSetHomeAtRegion; `fail`: the
  capability does not answer; `udp`: a region without the capability, the
  alert alone confirms; `forbidden`: the region has no
  REGION_FLAGS_ALLOW_SET_HOME, the menu entry is greyed. `:set` asks for it
  by itself at frame 700 (capture at about 780).
- **`AURORA_DEMO_USERINFO=hidden|liaison|opensim|fail|savefail`**: what the
  demo grid keeps of the account, for Preferences › Privacy › Visibility
  (with `AURORA_DEMO_OPTIONS=12`). Without it: seen online by everyone.
  `hidden`: "Only friends and groups know I'm online" ticked; `liaison`:
  ticked and locked (a value the checkbox cannot change); `opensim`: another
  grid, the e-mail checkbox with the demo address instead of the button to
  the account's page; `fail`: the settings never come, the switches stay
  greyed; `savefail`: a change is refused, a notice says so and the
  switches go back to what the grid has.
- **`AURORA_DEMO_EXPERIENCE=1|auto|profile|blocked|walk|arrive`**: experiences
  asking for permissions; the demo plays the simulator, GetExperienceInfo and
  ExperiencePreferences. In SL a script may ask from any event (a click, a
  collision, the arrival on the parcel, a timer): the viewer only sees the
  ScriptQuestion, shown whatever the cause, even for an object out of view.
  `1`: a violet "Club door" in front of the camera, above the avatar's head
  (136, 128.5), with the touch cursor. A click asks "Experience: Aurora Club
  Teleport": the object and its owner, the land-scope experience, "Teleport
  you", Yes / No / Block object / Block experience, and an "Experience
  profile" link that keeps the question. Yes teleports up to (150, 140, 60);
  once allowed, the next clicks teleport without asking. No: the door says
  it needs the permission. Block experience refuses and blocks: the next
  clicks only print "you have blocked this experience". The profile (the
  link) shows the rating, the scope, the location, the owner and the
  permission, with Allow / Forget / Block; Forget lets the door ask again.
  `auto` asks at frame 420 without a click (captures), `profile` also opens
  the profile; `blocked` starts with the experience blocked.
  `walk`: a translucent teal "Teleport pad" on the floor in front of the
  avatar (135.5, 126), a phantom volume: walking forward into it (W / up
  arrow) asks the same question (collision_start), once per entry; Yes
  teleports.
  `walk` and `arrive` place no door. `arrive`: the plaza's land-scope
  experience "Aurora Plaza Welcome" asks at frame 420 from a "Plaza greeter"
  the viewer has never seen (out of view, no click): "Animate your avatar"
  and "Change your environment settings"; Yes makes the greeter push an
  environment (PushExpEnvironment, PushPartialEnvironment): over 3 s the sky
  fades to a violet sunset and the water fog turns red. The profile's Forget
  fades it back over 1 s, Block at once (ENV_PUSH, `world/eep_push.rs`).
- **`AURORA_DEMO_LURE=offer|request|god|answers|reject|offer-card|request-card|card-answered|notification-answered|legacy|exceeded|exceeded-continue|blocked`**:
  teleports received from Tess Touch (issue #280, parts B1 and B2). Each
  offer and request opens her conversation (unread) with its card and has
  an entry in the notification centre; answering one closes the entry and
  leaves the card with its answer, greyed.
  `offer`, with the notification list open: "Tess Touch offers you a
  teleport", the place ("Location: Aurora Demo (128, 130, 25)") above her
  invitation, Teleport / Decline; nothing in the nearby chat. `request`:
  "Tess Touch is requesting to be teleported to your location", her
  message, "Offer a teleport?", Yes / No; Yes writes "Teleport offer sent to
  Tess …" in her conversation. `offer-card` / `request-card`: her
  conversation in sight with the card (teal edge, Teleport / Decline or
  Yes / No), and no toast. The offer's appended SLURL differs from the lure
  destination by 2 m horizontally and 1 m vertically (#899): each chat card
  should show only the destination link and retain the invitation text.
  `card-answered`: the offer accepted from the
  card: "Accepted" on the card, the list empty, the teleport starts and fails
  offline. `notification-answered`: the offer declined from its entry:
  "Declined" on the card. `legacy`: the card as one line of the legacy
  style, its buttons at the end. `exceeded` (part C): the demo account
  wants General and Moderate and the offer leads to an Adult plaza: the
  card and the entry explain it and offer "Change and Continue" / Decline.
  `exceeded-continue`: answered by "Change and Continue": the rating is
  asked for, then the teleport leaves (and fails offline); the card says
  "Accepted". `blocked`: the demo account is not adult: the card and the
  entry say the region is for adults only, without a button, and the
  sender gets "The Resident cannot visit this region." then the decline
  (in the log of the demo commands). `god`: no notification, the
  teleport starts at once and fails offline (two lines in the nearby chat).
  `answers`: her conversation is open with "Tess … declined your teleport
  offer" as a system line, without a notification; Nova's acceptance goes to
  her transcript only. `reject`: "Reject teleport offers and requests" is on
  for the session: an offer and a request arrive, nothing is shown (each
  sender gets the automatic answer). The demo writes transcripts only with
  `AURORA_DEMO_CHATLOG`.
- **`AURORA_DEMO_RADAR=1|tp|walk|friends|groups|profile`**: the People window.
  `1`: residents at every range in "Near you" (with
  `AURORA_DEMO_TAGS=1`, those of the name plates, several of whom have a
  profile picture). `tp`: three more residents in a region 10 km away, which
  the demo leaves "by teleport" at frame 600; with
  `AURORA_FPS_LIMIT=60`, they are greyed out at frame 780 and removed from the
  list at frame 1400. `walk`: the residents of `1`, and one more who
  enters the region every 30 s (from the 10th second of the session),
  60 m east of us, walks up to 6 m by the 20th second and back to 60 m,
  then leaves the region: turn on the alerts of the radar's options menu
  (and "Play a sound for the alerts") to see, hear and send to scripts
  each crossing of the region, the draw distance and the chat range, in
  and out; add `AURORA_DEMO_SOUND=1` to hear the sounds. `friends` and `groups`: these tabs.
  `profile`: the Groups tab and the "Group Profile" window of the first
  group.
- **`AURORA_DEMO_PEOPLE=visuals|visuals-stack|visuals-legacy`**: open the Family set's visual editor with a live plate preview. Family supplies a pink background and names, while the higher-priority Shop supplies a gold border and star badge replacing the friend heart. The same resident belongs to both, with a non-friend included. `visuals-stack` shows both set icons; `visuals-legacy` keeps classic world tags unchanged. Change priority, colour, switches, icon, preferred size and the global icon mode; inspect the live preview and world plates. Close the editor to inspect the nearby/friends/set lists, chat and minimap. Reduced plates still expand on hover; distance and crowd rules still apply. Demo sets remain in memory.

  For assignment, drag a resident's name from the chat, or a row from People, onto a real set pill or the selected set's member pane. The resident is added without leaving their other sets; internal pseudo-sets do not accept drops, and an inventory drag is left for its original destination. Close the editor to inspect the resolved world plate; change priority to check that one set can supply the border while another supplies the name/background. With `AURORA_DEMO_PROFILE=wolf:compact`, the profile shows the selected set badges side by side.

- **`AURORA_DEMO_PEOPLE=sets-import`** opens contact sets after merging a synthetic
  Legacy Viewer account file through the real import parser. Family keeps its
  existing colour and gains the imported membership; Networking is new and
  includes a non-friend. Demo sets remain in memory. People > Contact sets > gear
  and Conversations > Contacts > Contact sets share **Import contact sets…**.
  Choose `settings_friends_groups.xml` from a Legacy Viewer account directory,
  or an individual set export (XML, at most 16 MiB). Same-name sets merge members
  and retain their current settings; existing nicknames and global preferences
  stay in place. Cancellation and invalid files leave the current contacts intact.

- **`AURORA_DEMO_PEOPLE_FILTER=<text>`**: combine with a People scenario to prefill its search. With `rows` or `legacy`, `tesstouch` and `tesstouch Resident` must find the resident displayed as `Tess ✧`; `TESS` must find the same resident. An empty filter restores all rows; unrelated text shows no matches. This deliberately searches display, legacy and account names regardless of the chosen name format, while retaining contact aliases and RLVa anonymity.
- **`AURORA_DEMO_PEOPLE=rows|crowd|sheet|legacy|friends|groups|sets|recent|blocked`**: People › Near you in
  Aurora's look, with the residents of the People mock-up (names, ranges,
  states, arrivals), alone on screen. `rows`: the rich rows grouped by
  range (whisper 10 m, say 20 m, shout 100 m, beyond), Tess selected with
  her quick actions out as under the pointer. `sheet`: the window at the
  mock-up's 860 × 600 with Nova's sheet (the rights she has and the ones
  we grant her, what the radar knows); a click on a row of a narrower
  window widens it by the sheet, and its cross gives the width back.
  `crowd`: forty residents in the compact density. `hover` and
  `crowd-hover`: `rows` and `crowd` with a friend under the pointer
  carrying every badge. In both densities every quick action stays visible
  at the right edge, replacing the time and distance during hover. Badges
  stay to the left of the actions and fold when necessary. Row heights stay
  at 42 points in rich density and 24 points in compact density, at every
  width and with or without hover. Actions stay on one horizontal line;
  actions overlay the names without changing their normal text layout.
  Button widths tighten when needed, with unchanged icon sizes and the
  avatar still available for row clicks. The
  details action before More opens the resident sheet.
  `legacy`: the same
  residents in Firestorm's table, for this run only (the preference is
  not changed). Capture `crowd` at frame 900 or later: the profiles
  (payment info, notes) arrive four a second.
  The other tabs, with the mock-up's contents and windows (the contact
  sets, the nickname, the pinned group and the recent people are set for
  the run only, nothing is written to the account's files): `friends`
  (860 × 600: Nova and Orion online, pixelboutique and testtouch offline,
  the rights each way on the rows, Nova's sheet with "Nickname and contact
  sets"), `friends-symbols` (`friends` with the contact sets of `visuals`:
  a set's symbol after the name where the set has one, its colour's dot
  otherwise, #1104), `groups` (860 × 500: "No active group", Aurora Builders active,
  pinned, two notices unread and its sheet open, the market's chat
  blocked, "3 groups out of 42"), `sets` (520 × 430: Family, Clubbing and
  Shop, the wolf's nickname, a non-friend, Nova chosen with her quick
  actions out: move, take out, menu), `recent` (420 × 300: four residents
  met lately, how long ago and where) and `blocked` (420 × 300: a resident
  blocked in full with "Unblock" out, one for voice and text, an object
  for its sounds). Each tab keeps its menu of several people (Ctrl /
  Shift) and its sheet; a window too narrow for the six tabs shows the
  ones around the open tab beside "…".
  Append `:<width>` for a responsive check, for example `hover:300`,
  `crowd-hover:300` or `friends:360`. An explicit-width friends scenario
  starts with the list alone; its details action must still open the sheet
  and retain the normal window expansion. Repeat with `friends:620` and
  the default friends scenario beside its sheet. Every friend action must
  remain visible to the left of its permission cells.
- **`AURORA_DEMO_STATUSMENU=1|invisible`**: the menu of the status
  dot, open at frame 300. `invisible`: with this status chosen (hollow
  grey dot); only the display changes, no right is modified and nothing
  is saved.
- **`AURORA_DEMO_PARCEL_NAME=<text>`**: the parcel name shown by the
  place island and its menu, over the demo's own (and over
  `AURORA_DEMO_BARS`'s long one); `\n` stands for a line break. To check
  how odd names sit beside the region: accents, emoji, other scripts,
  letters from a fallback font, a trailing line break. The parcel's name
  and the region text must share one baseline.
- **`AURORA_DEMO_BARS=<words>`**: the states of the top and bottom bars
  for the comparisons with the bars mockup, for this run only (nothing is
  saved), with the mockup's long parcel name and, with the islands, its
  voice light (connected). Words, separated by commas:
  `normal` (the islands as set), `legacy` (Firestorm's bars), `compact`
  (the place island as wide as its content), `rights` (the parcel rights
  named when there is room), `nobug` / `bug` (the bug report button
  hidden / shown: the bug's own island, after the wolf's, goes away and
  the place follows the wolf 8 px away), `voice` (talking on the dock's Speak button, its level at
  62 %), `chatwide` (the chat island 560 px wide: the dock moves right, its
  buttons narrow at 1280), `read` (no unread conversation counted); and one menu opened at frame 300: `menu`
  (the place), `options` (right click on the place island), `favorites`,
  `star`, `status` (the account's picture), `emoji` (the chat island's
  emoji picker). The bottom: the chat island on the left, the dock centred
  (buttons 34 px, 32 at 1280), the local chat lines in pills above the
  chat island, all 8 px from the edges; the chat island's button carries the unread counter of the conversations (the dock has no Conversations button by default). With `AURORA_DEMO_TOOLBAR=drag`,
  frame 654: a toolbar button dragged along the dock. The favourites:
  `fav12` (the mockup's twelve, with their thumbnails), `nofav` (none: the
  empty state), `added` (eleven and this place, just added from the star:
  first in the list, as Firestorm orders a new favourite, and "Add this
  place" greyed out), `favsnap` (the twelve without item thumbnails: each
  picture is the snapshot of its landmark's parcel, the pin where the demo
  parcel has none or its request fails); where they live: `row` (the row under the islands),
  `folded` (the row folded to its tab), `rowdrag` (frame 654: the second
  pill dragged after the fourth), `dock` (the dock's Favorites button);
  with `favorites` the place island's menu or, with `dock`, the grid opens
  at frame 300, the mockup's row or tile under the pointer from frame 700.
  `chatname`: the newest resident's name of the chat pills clicked at
  frame 700, its profile opens (an object's name inspects the object).
  The balance (#1340): `nobalance` (hidden whatever is saved, with either
  bars: the account island keeps the picture alone, 8 px from the system
  island; the Legacy bars show no balance), `balancemenu` (the account
  island's right click, under the island from frame 300: "Show the L$
  balance" checked, then "Buy L$…").
  The system island: `systip` (its detailed tooltip shown, the network's
  last minute under its losses and bandwidth), `sysmenu` (its right click,
  under the island at its right end as the mockup draws it, from frame
  300), `tplist` (four earlier places put in the session's history before
  the arrival, the back arrow's list of them open at frame 300: the
  newest at the top with its arrival time, the place we are in lit; with
  `legacy`, under the full-width bar's arrow), `locations` (the place
  menu opened at frame 300, "Aur" typed in its destination field at frame
  320: the landmarks whose name holds it, whatever the case, and the
  history's places whose title holds it as typed, in dictionary order, the
  first one lit for Enter; with `legacy`, typed in the location field, the
  list under it; `tplist,locations`, in that order, for history places to
  match), `netgraph` (Firestorm's traffic indicator shown before the audio
  controls), `sysperf` / `sysnet` (Performance opened from the island at
  frame 300, by its frame rate / its network: the full view 8 px under the
  island, its left and right edges on the island's, down to the work
  area's bottom; the compact view where it stands); the clock (#453):
  `clocklocal` (local time instead of the saved time, so a click on the
  clock does not change it in that run), `noseconds` (without its
  seconds), `clock12` / `clock24` (12 / 24 hours instead of the saved
  format; Auto writes the `clock:` log line at startup with the system's
  12 or 24 hours). In demo the network's graphs show a synthetic minute of
  traffic (about 1.2 Mb/s in, waves), no packet lost. The hidden interface:
  `hidden` (hidden at frame 600 as Alt+Shift+U does: the bars and the
  windows go, the hint at the top in the middle for 6 s; with `legacy`,
  Firestorm's question instead), `peek` (hidden, the pointer held at the
  top edge: the top islands back, no hint under them), `peekleft` (hidden,
  the pointer held at the left edge: the left toolbar back, with
  `AURORA_DEMO_TOOLBAR=side`), `immersive` (#859: Conversations opened at
  frame 300, immersive mode at frame 600: the bars and the other windows
  go, Conversations and the chat lines stay, the avatars' name tags go
  (the objects' floating texts stay), no edge brings a bar back, no hint;
  faint square glass buttons in the top right corner, 8 px from the
  edges and as high as the islands: the name tags (struck-through label),
  the HUDs (#1190: violet screen, shown; no button while Show HUDs is
  off) and the way out), `immersivetags` (the same with the name tags
  shown: the label violet), `immersivehuds` (the same with the HUDs
  hidden: struck-through screen; with `AURORA_DEMO_HUDS=1` the demo HUDs
  go; nothing saved).
  The chat bar: `autohide` (shown only while it is used, as with
  Preferences › Chat › "Hide the chat bar when not in use", with either
  bars: with the islands the chat island goes and the dock gets its
  Conversations button back; with `legacy` the toolbar takes the whole
  bar), `chatstart` (autohidden, Enter pressed at frame 300 on the draft
  "Hello!": the island back, without its button, the keyboard in it).
  Capture at
  `AURORA_WINDOW_SIZE` 1280, 1536 and 1920 wide with `AURORA_FPS_LIMIT=60`
  at frame 780: at 1280 the place shows the parcel and the region, at
  1536 the position too, with `rights` the forbidden rights are named at
  1536 and all of them at 1920. With `AURORA_DEMO_NOTIFS=1`, frame 1160:
  the bell's red counter.
- **`AURORA_DEMO_CONTACTS=friends|groups|sets|detached|add|hidden|empty`**: Contacts
  tab of Conversations on Friends, Groups or Contact sets (demo contact
  sets and nicknames), the detached Contacts window, or the resident
  picker of "Add...". The detached window has the look of the tab it
  left (#961): People's friends tab in Aurora's look of People with the
  Aurora or Compact conversation style, the contact list otherwise (add
  `AURORA_DEMO_PEOPLE=legacy` to see it). These modes enable the optional Contacts entry for
  the run; `hidden` disables it with Contacts initially selected, showing
  the fallback to nearby chat. The saved preference defaults to off
  (Preferences > Chat > Show the Contacts tab).
    In Aurora's look, Friends has no display-options gear: search, profile
    pictures and friendship rights stay available. Contact-set lists always
    show pictures; names use the set's explicit UI-name appearance and
    priority. Legacy column/list options stay in the contact table's
    right-click Options menu, and search stays visible there too. The old
    hidden-search and column settings must not alter the modern pane;
    regressions cover saved settings in the workspace and detached window.
  In the Aurora and Compact conversation styles, test at full width,
  700 points and 460 points: a double-click on a friend starts their IM.
  At full width the detail panel is always present: retain the selected
  friend or choose the first visible friend, and show a placeholder when
  the list is empty (`empty` clears the demo friends). A single click
  switches the displayed friend. At 700 and 460
  points it only selects the friend. Hovering shows a contact details
  button immediately before More; it opens the same sheet as a long press.
  Permission cells stay in the same right-aligned positions with or without
  hover. Every quick action stays visible to their left, on one horizontal
  line. Rows always stay 42 points high while resizing in either direction.
  Actions paint over names without reserving any text width, even while
  hidden; their widths tighten if necessary and avatars remain clickable.
  Permission groups only show
  when room remains for every action, independently of hover.
  Details remains before More.
  When the sheet replaces the list, clicking
  Contacts returns to the list just like the sheet's cross, keeping the
  filter and selection. Repeat a double-click after returning to the list.
  Resize across 600 and 850 points in both directions: sort choices fold
  into a menu, contact sets wrap when needed, and every quick action stays
  visible. The list controls must stay inside their pane. Returning
  from the large layout to a reduced one restores the list and its selection.
- **`AURORA_DEMO_CHATLOG=im|find|history[:find]|offline|long:<n>[:scroll]`**:
  the conversation history without a grid. The demo is its own
  "grid": its files are in
  `config\demo\accounts\<demo account uuid>\chat_logs\demo\`, never in
  the folder of a real account, and the scenario rewrites them at each launch
  (without this variable, the demo writes and reads back no history). The
  windows are placed at a known size.
  - `im`: a conversation with Nova whose file contains two past
    days. What to see: these lines in grey, with their senders' profile
    pictures dimmed (this scenario gives Nova and us one), with a date
    marker at each change of day, then the new message in its normal
    colours; the file `Nova Example.txt` receives this message.
  - `find`: the same, then Ctrl+F (pointer over the window) and a word typed at
    frame 300 (the field empty at frame 303). What to see: the find field in
    the head's row in place of the picture and the names, "5 / 5" and the
    arrows inside it, the messages at their full height, the matches on an
    amber background and an amber stroke along the current match's message
    only (from its author's row to its last line, not the day and "New"
    lines above it). `find:none` types a word that is not there ("No match"
    in the warning colour, the arrows greyed), `find:legacy` the same in the
    Legacy style (a cross at the field's end), `find:narrow` a window
    narrow enough for the rail.
  - `history`: the History window on a file of 19,950 messages
    (page 200 / 200, the most recent); `history:find` types Ctrl+F and a
    word in it: counter "9976 / 9976" once the whole file has been gone
    through, last match in view.
  - `offline`: the three messages kept by the "grid" during the
    absence, returned by the request made after login, each one
    preceded by "(Saved …, …)", after the history in grey.
  - `long:<n>`: a conversation of `n` lines (2,000 at most, the limit),
    without writing anything, to measure the conversations view with
    `AURORA_PROFILE=1` (`u_chat` step); `long:<n>:scroll` scrolls the
    wheel upwards in it from frames 320 to 380, through lines never
    laid out.

  For these tests, which write files, prefer an isolated profile
  ("Isolated real installation" above: exe copied into
  `<root>\install`, `AURORA_UPDATE_TEST_ROOT=<root>`): the histories
  are then under `<root>\profile\accounts\`.
- **`AURORA_DEMO_REPORT_ABUSE=menu|avatar|object|chat|pick|send|missing|copyright|experience|experience-report`**:
  the Report Abuse window opens at frame 600, from Help › Report Abuse
  (`menu`), on Nova (`avatar`), on the nearest object (`object`, its name
  and owner come from the demo server), on a chat line of Nova (`chat`),
  or with the object picker armed (`pick`: the next click in the world
  designates). `send` fills the form and presses "Report Abuse" at frame
  700: the thank-you notice, and a `demo abuse report:` line in the log
  with what would have left; `missing` presses it on a blank form (the
  category notice), `copyright` with "copyright" in the details (the
  intellectual property notice). `experience` opens instead the profile
  of the demo group's experience "Sandbox doors": its "Report Abuse"
  button opens the window on its owner (Orion Example as the abuser) with
  "Experience id: …" in the details; `experience-report` opens that
  window directly at frame 640. Capture with `AURORA_FPS_LIMIT=60` and
  `AURORA_CAPTURE_FRAMES=780` so that the screenshot is in. Nothing ever
  leaves the demo; a real report is only made on the grid.
- **`AURORA_DEMO_PROFILE=wolf|nova|friend|self[:tab|edit|compact]`**: a
  profile window (tab 0 Second Life / Profile, 1 Feed / Web, 2 Favorites,
  3 Classifieds, 4 Real life / First life, 5 Notes). The window is the
  Aurora one unless Preferences › Aurora › Profiles is on the classic
  window. `:edit` opens our own profile in edit mode, `:compact` gives the
  window the width of the compact layout (380 px). With this variable the
  three residents of the scenarios have a profile picture, so a banner:
  the scenarios of the profile mock-up are
  - a friend: `friend` (Nova, Premium, a contact set if
    `AURORA_DEMO_CONTACTS` seeded one, the rights on the Permissions line of
    Info, three favorite places, two groups of which one in common, private
    notes);
  - a stranger: `wolf` (blocked: red badge, "Unblock"; "Add as friend" on
    the Friendship line of Info);
  - my profile: `self`, then `self:edit` (camera button, pencil, About
    editor, Visibility, the unsaved bar once About is changed);
  - compact: `wolf:compact` (icons in a row, the tabs in a drop-down list).

  `friend:photo` opens the main photo preview; `friend:first-photo` opens the
  First life photo preview. Append `:actual` to either to display native pixels
  with scrolling. Both previews offer the inventory texture preview's dimensions
  and original, preset or custom aspect ratios.

  Options add up, separated by `:` (`self:edit:2`). The other tabs:
  `friend:2` (Nova's favorite places, featured place over thumbnails),
  `friend:3` (two classifieds, the first one unfolded), `friend:4` (first
  life picture and text), `friend:5` (notes), `nova:2|3|4` (the empty
  states), `self:edit:2` (our places in their form, the card that adds
  one), `self:edit:3:form` (our classified in its form, with its clicks)
  and `self:3:publish` (a new classified and the strip that asks its
  price). Publishing takes the price from the demo balance; the demo keeps
  our classifieds for the run.
- **`AURORA_DEMO_FEED=<url>`**: the user name and `/?feed_only=true`
  are appended to the address; a `data:` URL can put them in a comment.
- **`AURORA_DEMO_LAND=<tab>[:role][:classic]`**: the pages of Aurora's
  window `overview`, `manage`, `people`, or the tabs of the classic window
  `general`, `covenant`, `objects`, `options`, `media`, `sound`, `access`,
  `experiences`, `environment` (or their index); Aurora's window opens a
  tab's section of "Manage", scrolled to and unfolded (`options` on "What
  visitors can do", `general` on "Sale and ownership", `covenant` on the
  overview); `people` asks for the object owners as Refresh does, `owners`
  also scrolls down to them; `environment-picker` and `experience-picker`
  open the settings picker (the demo library's skies, waters and days, also
  listed without `AURORA_DEMO_ENV_SELECT`) and the experience picker with
  its results (a grid-wide one hidden, those already listed hidden). The
  demo simulator applies an environment, a reset, a deed, a release, a
  reclaim and the returns, and answers with the parcel changed;
  `show-objects` asks the simulator to highlight the owner's objects (the
  plaza's pillars and orbs outlined in yellow), `scripts` opens Script
  info (four scripted objects, then two attachments in "My attachments").
  `scripts-descending` starts with largest memory first; `scripts-name`
  searches for the localized fountain name, `scripts-owner` searches for
  the seeded Aurora owners, `scripts-empty` shows no matches, and
  `scripts-attachments` opens the attachment table. Both tables sort by a
  clicked column (again reverses), search object/owner names without case,
  show matched/total counts and keep their own search/sort across Refresh.
  A filtered-out object must leave the selection before Locate/Return.
  Roles: `owner` makes
  the avatar the owner of the parcel (everything editable); `officer`
  leaves the parcel to the group "Aurora Builders" and gives the avatar
  its land powers except sale, deed, release and terraforming (the
  camera over the picture shows, "Edit the land" stays greyed with the
  power it needs, "View as" offers Officer / Visitor); `visitor`
  gives the parcel to another resident and to a group the avatar is not
  in (no camera, "Buy a pass" follows the parcel); without a role the
  parcel belongs to the group and the avatar has no powers in it.
  `:classic` shows Firestorm's nine tabs for the session without saving
  the choice of Preferences › Aurora. The demo parcel is residential, with
  a fixed landing point and building and object entry for its group, like
  the mockup; `AURORA_DEMO_LAND_PRIMS=10500` gives its objects gauge.
- **`AURORA_DEMO_LAND_PRIMS=<used>[,<step>]`**: with `AURORA_DEMO_LAND`,
  the objects of the owner's land in the region (out of 15,000); the
  gauge of the header is violet, orange when 20% or less is left
  (`12200`), red when 10% or less is left (`13900`, or `16200` beyond
  the capacity). With a step (`10500,450`),
  each answer of the demo simulator counts that many more: the
  gauge advances on its own at each refresh of the parcel
  (every 15 s with the window open, 1 s after an object is rezzed or removed).
- **`AURORA_DEMO_REGION=overview|manage|people[:owner|:manager|:resident][:preview][:compact][:access|:rules|:terrain|:scripts|:environment|:experiences|:covenant|:interventions][:pending][:ask-…|:top-…|:pick-…][:telehub|:raw|:pbr][:classic]`**:
  the Region / Estate window opens on the demo region, whose simulator
  answers with the mockup's figures: RegionInfo (limit of 40 avatars, 100
  at most, estate 104 123, no terraforming, pushing limited, voice, resale
  but no join / subdivide), SimStats (12 avatars, 10,500 objects of
  15,000, dilation 0.99, 45 frames, 4.2 ms of scripts for 312 scripts,
  9.8 ms spare of 22), the server version, the covenant, the region's day
  cycle ("Nordic Dawn", 4 h), its experiences (1 trusted, 2 allowed) and
  its restart schedule (Sunday at 06:00 UTC). `owner` (the default) makes
  the avatar the estate owner; `manager` gives the estate to another
  resident and makes the avatar its manager (RegionHandshake
  IsEstateManager); both get `estateupdateinfo` and the estate's lists
  (the header names the managers, shows the estate's access and voice,
  "Message to all…" and "Restart…" open their dialogs, "View as" offers
  the role / Resident).
  `resident` gets neither: no People tab, no "View as", no managers nor
  interventions. `preview` opens on "View as" Resident; `compact` opens
  the window 480 px wide (no map, one column, the action buttons as
  icons). `manage` opens on Manage
  (who can come, region rules, terrain and water, scripts and physics,
  environment, experiences, covenant, interventions): `access`, `rules` or `terrain` scrolls to
  that section, `pending` makes two changes there and scrolls to the bar
  "2 changes not sent yet" (Cancel / Apply), and `ask-override`,
  `ask-linden`, `ask-bonus`, `ask-maturity`, `ask-heights` or `ask-bake`
  opens that question (EstateParcelAccessOverride, ChangeLindenEstate,
  ChangeObjectBonusFactor, RegionMaturityChange, ConfirmTextureHeights,
  ConfirmBakeTerrain). The demo simulator keeps what Apply sends and
  answers it in the next RegionInfo / `estateupdateinfo`; the terrain's
  textures and elevations, which a real simulator sends back in a new
  RegionHandshake, stay shown as sent. A manager sees the bake greyed
  with the amber "estate owner" lock, a resident (or `preview`) the amber
  banner and a sentence instead of the estate's access. `people` opens on
  People, the window 840 px high: the four lists (2 managers, 3 allowed
  residents, the group "Aurora Builders", 2 banned residents with their
  ban date, who banned them and their last login). `select` ticks Orion in
  the allowed residents ("1 selected · Remove…"), `pick` opens the
  resident picker on them, `ask-ban` asks to ban Orion (this estate / all
  the estates you own) and `ask-remove` to remove him. The demo simulator
  applies each `estateaccessdelta`, answers `setaccess` and sends the
  lists again: what is added, banned or removed shows in the list. A
  manager sees the managers' list greyed with the amber "estate owner"
  lock; `preview` shows the card that says the simulator sends the lists
  to the owner and the managers only. `interventions` scrolls Manage to
  Interventions (message, teleport home, kick, return a resident's
  objects, restart and its cancel, the restart schedule), and
  `ask-message`, `ask-home` (Pixel chosen), `ask-kick` (Orion),
  `ask-return` (Orion, scripted objects on other people's land),
  `ask-restart` or `ask-schedule` (Sunday at 06:00) opens that dialog.
  The demo simulator answers a restart with the region's notice (the
  alert counts down and the header shows the red "Restart in m:ss" pill,
  a click on it cancels) and a cancel with "cancelled", and keeps the
  schedule it is sent; the message, the teleports home, the kick and the
  return are only logged ("sending estate request '…'").
  `AURORA_DEMO_RESTART=300` with `overview` shows the pill without a
  dialog. `scripts` scrolls Manage to Scripts and physics (disable
  scripts / collisions / physics, kept by the demo simulator in the next
  RegionInfo), and `top-scripts` or `top-colliders` opens the Top window
  on that report: five scripted objects with their time, memory and URLs
  (312 scripts in all), or three colliders; the filter by object, owner
  or parcel and the returns act on the demo simulator's list.
  `environment` scrolls Manage to Environment (the region's day "Nordic
  Dawn", 4 h, offset by -8 h, the parcels' own environments allowed; the
  Advanced fold has the day's length and offset, the three sky altitudes
  and the tracks' settings), `pick-env` opens the settings picker for the
  whole day (with `AURORA_DEMO_ENV_SELECT=1` for items to pick), `ask-env-reset` asks before going back to the default
  environment (SettingsConfirmReset) and `ask-env-override` before the
  parcels lose their own (EstateParcelEnvironmentOverride). The demo
  simulator keeps the day's timing, the altitudes, the item applied and a
  reset, and answers each with the region's ExtEnvironment; "Edit the day
  cycle…" opens the day editor on the region's day. `experiences` scrolls
  to Experiences (1 trusted, 2 allowed, 0 blocked, "n / 8" each),
  `pick-trusted` opens the experience picker on the trusted list (a search
  finds the region's experiences and a grid-wide one, which only the
  blocked list takes) and `ask-xp` asks where to remove "Treasure hunt"
  (this estate / all the estates you own); the demo simulator applies each
  `estateexperiencedelta`, answers `setexperience` and sends the lists
  again. `covenant` scrolls to Covenant, `pick-notecard` opens the
  notecard picker (two notecards are seeded in the inventory) and
  `ask-covenant` asks before clearing it (EstateChangeCovenant); a
  notecard dropped from the inventory asks the same. The demo simulator
  keeps the covenant it is sent and answers the next EstateCovenantRequest
  with it (the notecard's text, or "This estate has no covenant").
  `telehub` opens the region rules' Advanced fold, the window taller:
  the demo region's telehub "Arrival point" with two spawn points; the
  demo simulator applies Connect and Add the selection (a prim selected
  with the build tools), the x of a spawn point and Disconnect, and
  answers TelehubInfo. `raw` opens the terrain's Advanced fold: Download…
  saves a flat 256 × 256 RAW file where asked, Upload… reads one and the
  demo simulator takes it. `pbr` turns the terrain's slots to PBR
  materials (the bar counts 4 changes) with that fold open on the four
  materials' transforms, which the demo simulator keeps.
  `manage:pick-material` opens the first terrain material picker on the
  demo's tiled PBR material. Its sphere must load without a persistent
  "Loading…" label; OK keeps the selection and Apply validates all four
  slots. Add `classic` to check the same picker from the classic Terrain tab.
  `manage:pbr-ground` starts with a region already using PBR tiles (8 m
  repeats and a normal map), with all four GPU material swatches visible.
  `manage:pbr-preview` changes the first layer to a matte green material locally,
  leaving its picker open and Apply untouched; `manage:pbr-cancel` cancels
  that picker and restores the tiled ground; `manage:pbr-apply` commits it.
  All use the real preview/cancel/Apply handlers, offline. Compare captures
  at frame 760, optionally with `AURORA_DEMO_POS=120,40` for open ground.
  In either window style, selecting a texture/material must immediately
  change its swatch and the ground. The Cancel button beside the four swatches
  restores the committed cover without undoing water-height or elevation edits.
  `manage:pbr-mask` uses a holed PBR material on the ground: colour, depth,
  picking, sun and projector shadows must all respect its alpha mask.
  `manage:pbr-paint` creates the experimental local blend map from the advanced
  terrain fold and scrolls its controls into view; `manage:pbr-paint-reset`
  resets it while work is pending and shows the same controls.
  The map freezes blend weights locally, uses linear RGB with the first layer
  as the remainder, and never sends an estate edit. Reset returns to live
  height/noise composition. Add `classic` to check the same controls there.
  Bakes use 2048 px by default, or 1024 px on a small upload pool; workers
  prepare and stage all mip levels without a frame-thread image copy.
  `manage:pbr-slopes` adds a sharp mound and a hollow near `120,40` to check
  normal-map projection on curved terrain. Terrain uses the reference diagonal
  normal stencil and mesh-accumulated tangents, encoded in the existing 8-byte
  parallel vertex stream. Asset sampling converts GL UVs to decoded top-down
  JPEG2000 rows; local blend maps keep their own region-space row convention.
  `classic`
  opens the classic window (the Legacy Viewer's eight tabs) on the tab of
  the section asked (`terrain`: Terrain, `scripts` or `interventions`:
  Debug, `people`: Access, none: Region).
- **`AURORA_DEMO_GROUP_PROFILE=1|member|officer|owner|guest[:members|:invite|:notices|:compose|:admin|:identity|:land|:experiences]`**:
  the group profile, as after a click on the group or the owner in "About
  Land". A simulated group server (`demo_group.rs`) answers the members
  (1,248, five of them online), roles, role pairs, titles and notices of
  "Aurora Builders". `1` or `member`: we are one of its builders (Chat,
  Leave, "Me in this group"); `officer`: an officer, who sees Invite and
  the Roles and Donation columns; `owner`: an owner, every ability;
  `guest`: the profile of "Boreal Photographers", a group we are not in,
  with an open enrollment and a L$ 250 fee: "Join · L$ 250" asks to
  confirm (or says the balance is short), then the group joins our list
  and the window becomes a member's. `:members` opens the Members tab with
  a builder selected; `:invite` opens "Invite to Aurora Builders" with
  three residents, one of them already a member: Send Invitations logs
  `group invite: 2 resident(s) invited` and says the member was left out;
  the role list follows our abilities (an officer: everyone and the
  officers' own roles; an owner: every role, the owners' asking to confirm).
  With `officer:members` or `owner:members`, the Members tab manages:
  tick members (Ctrl+click adds), Eject and Ban follow Firestorm's rules
  (an officer ejects a member in no role but "everyone", an owner anyone
  but another owner, never ourselves) and ask to confirm, the log says
  `demo group: N member(s) ejected` / `ban action`; the roles card ticks
  the roles we may give or take (a tick waits for Save, adding owners asks
  to confirm, `demo group: N role change(s)`); "What they can do" lists
  the abilities of the chosen by set; the download icon exports the list
  to a CSV file.
  `owner:admin` (or `officer:admin`) opens the Administration tab: Roles
  (New, Copy, Delete within 10 roles, name, title, description, members
  visible, abilities by set, the dangerous abilities asking first; Save
  logs `demo group: N role update(s)`), Group identity (insignia from the
  texture picker, charter, show in search, moderate content, open
  enrollment and fee; Save logs `demo group: identity saved` and the
  simulated server keeps it: a new insignia shows at once in the window,
  the conversation list and People › Groups), Abilities
  (who has the ability chosen), Banned residents (two at start; Unban,
  Ban residents by name), Invitations. `owner:identity` opens it on Group
  identity (to compare with the creation window).
  `:notices` opens the Notices tab with the two latest notices unread
  (the tab's count): the first one is read at once (GroupNoticeRequest,
  answered by an IM_GROUP_NOTICE_REQUESTED instant message), its
  attachment "Save to inventory" logs `demo group: notice attachment
  accepted`; `:compose` opens a new notice (an officer may send notices):
  a copyable and transferable item dragged from the inventory attaches,
  a no-copy one or a worn attachment is refused, and "Send to 1,248
  members" adds the notice to the list and brings it back as a group
  notice in the notification center.
  `:land` opens the "Land and L$" tab (`:land-details` and `:land-sales`
  on those pages of the accounts): 4,608 m² in use out of 5,120 given,
  two parcels (Sandbox on Aurora Demo, Workshop on the sandbox region)
  with Map, About Land (only for the region we are in) and Place profile
  on a right click; our contribution of 512 m² out of the 1,024 we may
  give (a new value applied with Enter logs `demo group: land contribution
  set to N m²`, a value over the maximum goes back); the planning page
  (balance, credits, debits, total, next stipend day), the details and
  the sales of this week and of the eight weeks before (the arrows; the
  sales arrive in two packets, both shown; nothing after six weeks).
  `:experiences` opens the Experiences tab (its count, 1, shows on the tab
  as soon as the window opens): "Sandbox doors", General, owned by the
  group, land-scope; "Profile ›" opens the experience profile, whose
  Allow / Forget / Block log `demo group: experience permission …`.
  Check the header against the mockup (banner,
  insignia, badges, tabs), the switches of "Me in this group" (the active
  group follows in the top bar, the title in the Title shown list), the
  filter, the pills and the column sorts of Members, "Leave" (confirmed,
  then the guest's view) and, in Preferences › Aurora › Profiles, the
  classic window with its "Members and roles", "Notices", "Land and L$"
  and "Experiences" sections.
- **`AURORA_DEMO_GROUP_CREATE=form|cost|refused|limit|created[:classic]`**:
  "Create a group", as from the "Create" button of People › Groups, once
  the group list is in. The login of these scenarios gives a Base
  membership whose group creation costs L$ 100 (Premium L$ 50, Premium
  Plus L$ 0, listed in the fee card) and a limit of 42 groups. `form`: the
  empty window (Aurora look: the profile's header with the name typed in
  it and the insignia to click, the charter and settings cards of
  Administration › Group identity, the fee card); `cost`: "Lagoon
  Painters" filled in and Create clicked: the L$ 100 confirmation; `limit`:
  the account at its limit (its three groups): "You have reached your
  maximum number of groups" instead of the window; `refused`: "Aurora
  Builders" sent: the simulated server refuses the name ("Unable to create
  group." and its reason), the form stays; `created`: "Lagoon Painters"
  sent: the server takes L$ 100 from the balance (top bar), the group joins
  the list (People › Groups) and its profile opens, us its only member and
  owner. In the window, try a name of three characters (the 4 to 35 rule),
  an accent or a leading space (refused as typed, the rule in amber),
  Create without a content rating (Yes / No / Cancel), then confirm the
  fee. The server also refuses a name already created during the run, and
  a fee above the balance. `:classic` shows Firestorm's creation panel
  (Group Name, insignia and charter, Anyone can join, Cost to join, the
  maturity list, the fees by membership, Create group for L$ 100, the
  48-hour note), and the new group's classic profile.
- **`AURORA_DEMO_LAND_MODERATION=menu|none|freeze|eject|eject-ban`**: the
  violet wolf stands on our land (the ParcelOverlay cell under it becomes
  ours), then at frame 600 `menu` opens its world menu from its name plate
  (capture 720 with `AURORA_DEMO_PIE=0` and the pointer on Disturbance,
  `AURORA_DEMO_POINTER="830,585,s"` in the default window): "Freeze" and
  "Eject" follow "Report". `none` leaves the land someone else's: the two
  entries are hidden, as in the Legacy Viewer. `freeze` asks Freeze /
  Unfreeze / Cancel; `eject` asks Eject / Cancel and `eject-ban` adds
  "Eject and ban" (a parcel whose ban list is ours). An answer logs the
  FreezeUser or EjectUser command (`land moderation:`); the demo simulator
  does nothing with it. On the grid, test as the parcel owner, an estate
  manager and a group member with and without "Eject and freeze": the
  resident must be frozen, unfrozen, ejected, or ejected and banned.
- **`AURORA_DEMO_SOCIAL_MENU=menu|picker|invite|new-set|new-set-done|particles|card|card-offer`**:
  a resident's "Invite to group", "Add to contact set", "Block
  particles" and "Give my calling card", on the wolf (unblocked for the scenario, owner of the
  plaza's fountain, and our abilities in "Aurora Builders" an officer's).
  Capture with `AURORA_FPS_LIMIT=60` (and `AURORA_DEMO_PIE=0` for the list
  menu). `menu`: the wolf's world menu at frame 600, from its name plate
  (capture 680); add `AURORA_DEMO_POINTER="886,344,s"` (default window)
  and capture 720 to see the submenu: each contact set, then "New contact
  set…". `picker`: "Invite to group" at frame 600, the groups where we may
  invite (only "Aurora Builders", in bold as the active group, OK greyed
  until one is chosen; capture 680). `invite`: the picker answered at frame
  640: "Invite to Aurora Builders" with the wolf in the list and the roles
  (capture 780), without the group's profile. `new-set`: "New contact
  set…" at frame 600, its name asked (capture 680); `new-set-done`: named
  "Pack" at frame 640, People › Contact sets on it with the wolf (capture
  780). The scenario keeps the contact sets in memory: the demo account's
  file is left as it was. `particles`: capture 690 shows the fountain's
  particles; "Block particles" at frame 700 makes them go at once, and the
  menu opened at frame 720 shows the entry checked (capture 780); the log
  says `demo social menu: particles of … blocked: true`. This uses the same
  independent local owner filter as the contextual Particles submenu, with
  no mute entry or effect on the resident's avatar or communications. `card`: "Give my
  calling card" at frame 600: "You have offered a calling card to …", then
  the demo server accepts it, "Your calling card was accepted." (capture
  640). `card-offer`: the wolf offers us his calling card at frame 600,
  Accept / Decline (capture 640); the answer is logged as `demo social
  menu: calling card accepted|declined`.
- **`AURORA_DEMO_PLACE=1|lagoon|nordheim|grove|flaw|unknown|landmark|history`**:
  "Places" on a place profile, as after a click on a place link.
  `1`: the home of Violet Wolf on Aurora Demo (the parcel of "About
  Land"); `lagoon`: a Moderate parcel; `nordheim`: an Adult
  parcel belonging to a group; `grove`: a parcel with no name,
  description or picture; `flaw`: RemoteParcelRequest answers HTTP 404;
  `unknown`: a region for which nobody answers (error after 10 s);
  `landmark`: the profile of a landmark; `history`: that of an entry of the
  teleport history.
- **`AURORA_DEMO_PLACES=favorites|landmarks|history[:wide,open]`**: the Favorites
  and Landmarks folders of the demo (with a subfolder) and a teleport
  history over several months. In Aurora's look (Preferences › Aurora ›
  Places), `:wide` opens the window at 1040 × 640 (the rail of sources and
  folders, the list, the preview) and `:open` selects its first place (a
  narrow row opens in place); `:wide,open` does both. The demo's favourite
  of the lagoon is a copy of its landmark (same asset): one place with a
  lit star.
- **`AURORA_DEMO_LANDMARK=create|favorite|landmark`**: once the avatar
  stands on its parcel, `create` opens World › Landmark this place: the
  landmark is created at once, named after the parcel; OK renames it,
  gives it its notes and moves it to the chosen folder, Cancel deletes it.
  `favorite` / `landmark` make a landmark of the plaza in Favorites /
  Landmarks, as the star's menu does: the star at the right end of the
  location field turns amber (its tooltip names the folder and the
  landmark), and a click on it edits that landmark. World › Landmark this
  place and the Places creation entries stay available: save several
  landmarks without moving, including from a Places folder, and check that
  each creates a separate inventory item. Without the variable the star is
  an outline and opens its menu. The demo parcel is the 128 m square around the plaza
  (x and y 64..192): a landmark made there lights the star, wherever in it
  the avatar stands. With `AURORA_DEMO_PLACES` the folder list shows the
  demo's subfolder.
- **`AURORA_DEMO_SEARCH=all|web|people|groups|places|land|events|classifieds[:text]`**:
  the "Search" window on this tab (Aurora's look: on this chip, `all`
  being its "All" chip that searches the five text directories at once),
  the search for the text already run (`aurora` by default): the list is
  filled by the directory of the demo, the first row is selected and its
  details shown on the right. In Aurora's look, visible group rows request
  their profiles and insignias and keep initials when no image exists.
  Both layouts replace directory member counts with the current profile
  or completed member list, including the sort order. Join
  uses the profile's fee, balance and group-limit confirmation in both
  styles; invite-only groups and existing memberships cannot be joined.
  Navigation chips wrap within narrow windows, including their counts.
  In Aurora's look the rows of places, land
  sales and classifieds show the demo snapshot and their region once
  their details answered. For `land` in the Legacy look, the text is the
  price limit (`land:25000`); without it the field keeps its default "0"
  and only the auctions (L$ 0) pass the limit checked by default; Aurora's
  land filters start empty (no limit).
  A text containing `page` (for example `people:page`) answers a full
  page: "> 100 found" and the Next button; a text containing
  `xyz` finds nothing ("'xyz' not found").
- **`AURORA_DEMO_SEARCH_WEB=<page>`**: the address the Web tab loads in
  demo (a local `file:///…` page is enough to try back, forward,
  reload and the address). Without it the Web tab of the demo loads
  nothing: the search page of the grid is only opened once
  connected.
- **`AURORA_DEMO_GRID_HELP=<page>`**: Help › Knowledge base shows in demo
  as on an OpenSim grid whose grid info gives this help page, with the
  page's host as the grid's name at the right end of the row. Without it
  the entry is hidden, as on Second Life.
- **`AURORA_DEMO_MARKETPLACE=<page>`**: World › SL Marketplace stands for
  this page in demo (a local `file:///…` page is enough): the window opens
  at start on it, the menu entry shows, and a link to an address under it
  opens in the window instead of the system browser. Without it the demo
  has no Marketplace (the entry is hidden, as on a grid without one).
- **`AURORA_DEMO_MARKETPLACE_LISTINGS=merchant|not-merchant|error|audit|menu-listing|menu-version|menu-item`**:
  Me › Marketplace listings open at start, answered by an SLM server
  simulated in the viewer (nothing is sent). `merchant` (any other
  value too) adds sample listings to the inventory: a listed hoodie with
  two versions, an unlisted poster sold from a stock folder of three
  no-copy items, a lantern folder never associated (loose items, which
  the audit reports), an unfinished listing the server refuses to list
  (the 422 alert), and a "Products to sell" folder to drag in.
  Listing a listing without stock leaves it unlisted (the "could not
  be updated" alert). `not-merchant`: Firestorm's explanation and the
  link to open a store. `error`: the connection error with its reason.
  `audit`: `merchant`, with the audit window of every listing open.
  `menu-listing`, `menu-version`, `menu-item`: the shared inventory context
  menu open on a listed folder, its active version or an object in it.
  Check rename (including F2), properties, image, folder protection/reload,
  folder windows, shared copy/cut/paste, links, delete, grouping and favourites.
  Changes to an active listing ask for confirmation; paste checks sale
  permissions, stock compatibility and hierarchy limits again when confirmed.
- **`AURORA_DEMO_PLACE_WINDOW=1`**: with `AURORA_DEMO_PLACE`, the separate
  place windows instead of "Places" ("Landmarks and place
  profiles" option, not saved).
- **`AURORA_DEMO_TP_CONFIRM=link|landmark`**: the confirmation of a
  teleport requested by a teleport link (`link`, the default)
  or by a landmark of "Places" (`landmark`), with its "Do not show
  again" checkbox. The question is asked even if the demo setting
  (Preferences › Interface › Teleport "Confirm teleports")
  is disabled; checking the box then "Teleport" disables it in the
  settings of the demo, separate from those of the connected viewer.
- **`AURORA_DEMO_RESTRICTED=1`**: red parcel icons, damage
  enabled, health at 72%. Voice is forbidden too: the microphone button
  is greyed and does not switch on (its arrow still opens the options);
  with `AURORA_DEMO_TALK=1` the open microphone goes off on arrival.
- **`AURORA_DEMO_FREEZE`**: at frame 660 Nova Demo freezes the avatar for
  30 s (`1`) or the given seconds: the alert in the chat and the
  notifications, a red snowflake in front of the parcel icons of the
  navigation bar; at 60 fps the release comes after that many seconds
  (frame 660 + 60 × seconds), with its own alert, and the snowflake goes.
- **`AURORA_DEMO_SIM_CONSOLE=1`**: the demo region gives its console
  (SimConsoleAsync): Advanced › Region console opens it (the Advanced menu
  shown, Preferences › Aurora); `help`, `stats` and `time` answer, any other
  command gets "Unknown command"; the arrow keys walk the typed commands.
  `open` opens it at frame 600 with `help` and `stats` typed (capture at
  frame 640).
- **`AURORA_DEMO_GRID_STATUS=open|new|none|error`** (`1` is `open`): the
  Grid status button at the end of the bottom toolbar, for this run only,
  and a feed served from memory, shaped like the status page's: an incident
  in progress (amber), a maintenance scheduled (violet) and two incidents
  resolved (green), dated from now, each with its newest update and the
  earlier ones folded. `open` opens the window at frame 600 (capture at
  frame 640): "Ongoing incident on the grid" at the top, Refresh and
  "Open the status page" on the right. `new`: at frame 700 a new incident
  ("Group chat messages delayed") comes in the feed; the button flashes
  eight times (about 4 s), then stays tinted until the window is opened,
  where the incident is first and marked "New". `none`: a grid without a
  feed ("This grid does not publish its status."); `error`: a feed that
  cannot be read ("The grid status could not be loaded." and the reason).

**`AURORA_DEMO_WEARABLE=<type>[:mode]`** opens the shared editor on a worn
skin, hair, eyes, shirt, pants, shoes, socks, jacket, gloves, undershirt,
underpants, skirt, alpha, tattoo or universal item. Optional modes are
`readonly`, `nocopy`, `dirty` (the unsaved-changes dialog), `preview` (a real
local texture/mask change), `save-as`,
`save` (a changed asset saved through the normal offline acknowledgement at
frame 650), `drag` (an inventory texture carried over the first texture row
from frame 700, without a press: capture at frame 712 for the violet outline)
and `drop` (the same, released there at frame 720: the swatch, the avatar
preview and Save follow; capture after frame 900). Capture after frame 760 for
save feedback. Check texture slots,
shared clothing tint, sex-filtered parameter groups, the fixed save controls,
the picker and its permissions, cancellation, and the existing shape/physics
editors when switching. No grid traffic is used.
`tattoo:local` applies a private RGBA checker to all three tattoo slots through
the normal local import publication. `tattoo:local-picker` opens its Local tab
at frame 700. `tattoo:local-reload` replaces the violet checker with a green
one at frame 900 using the importer's reload generation; capture before and
after frame 1100. `tattoo:local-cancel` cancels at frame 1000 and restores the
server appearance. These modes also accept the other wearable types. Save
and Save As stay disabled until inventory textures replace the local maps.
`tattoo:picker-live` opens the Local picker on an empty head slot and selects
its image with a mouse click at frame 700 without confirming: the avatar and
swatch must change while the picker remains open. `tattoo:picker-cancel`
clicks Cancel at frame 1000; `tattoo:picker-close` clicks the title-bar close;
capture at 850 and 1200: Cancel restores the preceding image, while closing
keeps the selected image on the avatar and the swatch. These picker modes
also accept `skin`, `hair`, `eyes`, every clothing type, `alpha` and `universal`.
Ordinary inventory picker clicks preview immediately too; OK confirms the
choice, the title-bar close keeps applied choices and explicit Cancel restores
the original slot. Closing the editor then asks Save/Discard/Cancel for the
unsaved draft; keeping the picker choice does not save the wearable asset.
Leaving an editor idle with parameter groups open must not change its weights
or restart its bake generation: fractional slider percentages are formatted
without rounding the stored asset until a real edit. Test skin and clothing
texture choices with their parameter groups still open.
The local preview composites the avatar definition's original masks and
worn layers on workers and uploads a complete generation through GPU staging.
It uses resident source pixels immediately, including BC7-only cached textures,
and rebakes when finer source levels arrive. Unrelated worn assets do not block
an edited region. It uses 2048-pixel bakes (512 for eyes), or 1024 on a low-memory staging pool;
saving still requests the server's normal bake. Capture `skin:preview`,
`shirt:preview`, `tattoo:preview` and `alpha:preview` after frame 900; cancellation
restores the pinned server textures. `AURORA_DEMO_INVENTORY=aurora-menu-folder-uuid`
shows the UUID command in the unlocked menu's hidden shelf;
`aurora-menu-folder-uuid-restored` shows it after the user exposes it.
`AURORA_DEMO_INVENTORY=aurora-menu-textures-export` selects a texture and a
snapshot and opens the context menu's export format group. Check both PNG and
TGA choices; each keeps the whole selection and uses one destination folder.
Exports retain full resolution, RGBA pixels and the existing full-permission
requirement, and refuse to overwrite existing files.

**`AURORA_DEMO_PHYSICS_EDIT=1|new|drag`** (with `AURORA_DEMO_PHYSICS=1` for
the balls and the hops): `1` wears a physics layer with the lively values of
`AURORA_DEMO_PHYSICS` and opens its editor, Breasts bounce open; moving a slider
changes the balls' bounce at once, Cancel restores it. `new` runs the outfit
gallery's "New physics": a "New Physics" layer is created, worn (it shows in
Appearance › Worn) and its editor opens; every Max Effect is at 0, so the
balls follow the body rigidly. `drag`: the same, then four seconds after the
editor opens every slider moves in two seconds to the lively values, as a
hand would (log: `demo physics edit: sliders at the lively layer`); the
`demo physics: frame …` offsets of `own` go from zero to a bounce while those
of `other` keep bouncing. Save stores the asset and applies it as the
appearance, as a simulator bake would.

**`AURORA_DEMO_SHAPE=body|head|eyes|ears|nose|mouth|chin|torso|legs|male|readonly|nocopy|dirty|save-as|saving|save-feedback|mesh-head|static-head|pbr-head|mesh-drag|layered-head|t-pose`**
opens the editor with a synthetic worn shape, without reading a real inventory or
connecting to a grid. Category modes select the corresponding controls; `male`
selects the male shape, `readonly` removes Modify, `nocopy` removes Copy, `dirty`
opens the unsaved-changes confirmation and `save-as` opens the copy name dialog.
`saving` holds a synthetic save pending, and `save-feedback` shows a wrapped
save error. In both, the editor must keep its size, with the three buttons
reachable at the bottom; only the parameter list gives space to the status.
`mesh-head` and `static-head` wear a tinted head through the real mesh codec,
with/without skin weights, using BOM so the system head is hidden in the world
and in the hints. Check the same head in both views and change Head Size. Wheel
over the category row goes forward/backward, stays within Body/Legs and does not
scroll the parameter list. The self-avatar/attachment menu's Appearance › Edit
Shape opens this editor. Asset arrivals and AO cycles must keep ready hint images
visible, with no repeated "Preparing preview" placeholders.
The right-aligned T-pose switch beside the height units shares the saved
customise-animation setting in Preferences › Avatars › Animations. Toggle it in either place while editing:
the other control must follow immediately, the choice must survive reopening,
and the shape must stay unmodified. On the grid, disabling releases the editing
animation; enabling uses the existing stand-up/customise animation sequence.
`t-pose` supplies synthetic pelvis turn/return motions and starts with the switch
on. Toggle it both ways: the camera must ease around the avatar over a half-turn,
keeping its distance and the edited part in view. Reverse it mid-transition to
check that the new arc starts at the displayed view. Category changes and the
wearable editors use the same orbit framing; normal camera modes keep their paths.
`pbr-head` uses a PBR skin material over a conflicting blue texture-entry tint;
the head must retain its textured skin color in the world and in the hints.
`layered-head` adds a cosmetic shell above a rigged, BOM-textured mesh head.
The shell uses the built-in transparent texture, which has no cached asset.
The hints must retain the skin color, as the world does; a white face means
the reserved transparent pixels were lost. Blended layers must not write depth
over the skin, regardless of their order in the mesh.
`mesh-drag` sweeps Head Size for six seconds, starting two seconds after opening:
the live head changes smoothly, existing comparison images remain visible and
the log has no hint dispatch during the sweep (one refresh after release).
Switch Body / Head / Torso / Legs: the main camera smoothly frames each area.
Combine with `AURORA_DEMO_CLASSIC=1` for a baked system avatar. Move a slider or
type a value, switch sex, use minimum/maximum
hints, filter parameters, toggle hints and height units, then Cancel, Save and Save
As, then reopen the editor to check the saved values. Save receipts and appearance
updates are simulated only in demo mode. Inspect the preview's framing,
the persistent footer and the same theme as Appearance; Cancel restores the latest
server appearance. `--language fr|en`, `AURORA_WINDOW_SIZE` and captures after frame
900 cover both locales, compact and wide layouts, and asynchronously loaded hints.
On the grid, also check a rigged body, real baked textures and persistence after
relogin; only the human connects and performs these checks.
Opening the editor stands our avatar up (a seated avatar leaves its seat, as
in the Legacy Viewer), and while the editor is open our avatar plays the
customise animation (not the physics layer editor, which keeps the camera,
like the Legacy Viewer's): the log
says `customise animation: start 038fcec9-…` on opening and, on closing, its
stop then `start 6883a61a-…` (the "done" animation). In demo the animation
is listed on our avatar but has no motion (no built-in animation assets).
The animation turns the avatar half a turn: once its motion is loaded (on
the grid), the camera frames it from behind its root as drawn, which is
then its front, again 2 s later on the pose the turn ends with, and the
hints drop the pelvis turn of their frozen pose. Without the motion (demo)
the camera stays in front of the root. As in the Legacy Viewer the avatar
is seen straight from the front, in the middle of the window, the whole
body from about 15° above, the head nearly level, torso and legs level;
it moves beside the editor only when the editor covers the middle of the
window (a narrow window). Preferences › Avatars ›
Animations › "Play the customise animation" (on) turned off: no stand up,
no animation, the camera in front; changing it while the editor is open
applies at once. Every 10 s the log says `shape hints: built again for
bakes <n>, worn meshes <n>, textures <n> times in 10 s; left alone: rigid
meshes moved <n>, face tints changed <n> times` when anything changed; on a
still avatar the rebuild counts stay at 0: the levels the texture budget
moves above the hints' 256 px, scripted prims that move and faces whose
tint, glow or texture placement a script changes do not rebuild the hints
(they are taken up at the next rebuild), a face hidden or shown does.

**`AURORA_DEMO_AVATAR_HEALTH=undeform|refresh`** runs one entry of Me ›
Avatar health at frame 600, as its menu row would. `undeform`: our skeleton
and animations reset, then the undeform animation started; the log says
`undeform: …`. `refresh`: every object we wear from the inventory asked again
on its point; the log says `refresh attachments: <n> of <m> worn objects
asked again`. The demo's worn objects (`AURORA_DEMO_WORN`, outfits) carry
their item id as object id, which is how a temporary attachment looks, so
the demo always logs `0 of <m>`; the choice of objects is covered by the
unit tests, the real requests by a grid test. The menu itself:
`AURORA_DEMO_DOUBLECLICK=menu,<y>` with the height of the Avatar health row
opens its submenu.

**`AURORA_DEMO_OUTFIT_RESTORE=warm|cold`** — the login restore of the
outfit's attachments (`world/outfit.rs`) against a demo region that behaves
like the busy region of the 2026-10-27 grid logs: it puts back by itself
the mesh hair 3 s after arriving and the boots 8 s after, refuses a request
for an object it is still putting back ("Attempt to rez an object failed")
and takes off an object added on a point before its own object arrives
there. The ring of the demo outfit is worn on the boots' point for this.
The region's 5 s pause is longer than the restore waits for it to be
quiet, the unfavourable case. What to look for in the log: `outfit: the
region put back 1 of 5 objects by itself`, `outfit: wearing 4
attachments`, the alert, `'Bague argentée' came off` (or the English name)
when the boots arrive, `outfit: wearing 1 taken-off attachments` about 4 s
later and `outfit: 5 of 5 attachments worn` about 30 s after the requests;
the Appearance window then opens on its Worn tab with the five objects
worn. `cold` hides the five items from the inventory until FetchItems asks
for them (`5 to look up`), as a cold inventory cache.

**`AURORA_DEMO_APPEARANCE=gallery|gallery-fit|gallery-fill|outfits|multi-gallery|multi-outfits|multi-items|multi-worn|folders|folders-outfits|folders-many|folders-many-outfits|worn|save|edit|layers|layers-drag|layers-hold|wearables|wearables-outfits|wearables-edit|menu|duplicates|broken|take-off-folder`**
— opens Appearance with four synthetic outfits:

- `gallery`: the gallery (the right click includes rename, favourites, image
  choice, save to the chosen outfit and move to the Trash);
  in both `gallery` and `outfits`, right-click a single outfit and choose
  **Copy outfit list to clipboard**, immediately before **Delete outfit**.
  Paste into a text field or notecard: first the item names alone, one per line;
  then, after one blank line, the same items as `item name - creator`, in the
  same order. The separator is a simple ASCII hyphen in every language, without
  headings or a Creator label. Both lists include body parts, clothing,
  attachments and gestures.
  Copy remains available on the worn outfit, where Delete is hidden; it is
  disabled while the outfit loads or when it has no usable items. Broken
  links and folder links are skipped. Creators come from the original item,
  not its outfit link. Missing names are requested through the avatar name cache;
  a short Preparing outfit list status shows while they resolve. After three
  seconds, an unanswered creator lookup uses its UUID; an item with no creator
  reports unknown / multiple. Closing Appearance still completes the copy;
  changing sessions cancels it. Inventory menus have no such action.
- `gallery-fit` / `gallery-fill`: local portrait, landscape and square preview
  cards. Fit shows the whole image; Fill uses a centred crop without stretching.
  The discs stay round, the images stay inside their frames and the names and
  Worn tags stay below. Toggle **Fill outfit previews** in the gallery's bottom
  options menu; this setting is saved and off by default. Reopen Appearance
  and restart with `gallery` to check persistence without a forced mode;
- `multi-gallery` / `multi-outfits`: two selected outfits in the gallery or
  list. Ctrl/Cmd-click toggles one outfit; Shift-click selects a range in
  the current filtered/sorted order. Right-clicking a selected outfit keeps
  the selection. Wear replaces the current clothing with the combined
  selection; Add keeps the current outfit. Both use one COF mutation,
  deduplicate shared items and preserve required body parts. A combined
  replacement has no single saved base; use Save as to save the combination.
  Conflicting body parts use the last selected outfit in display order,
  following `LLAppearanceMgr::filterWearableItems`.
- `multi-items` / `multi-worn`: the second outfit open in the list with
  two of its items selected, or two worn items selected in Worn. Ctrl/Cmd
  and Shift pick items within one outfit or within Worn; a click in
  another list starts a new selection there. Right-click a selected item:
  Wear, Add, Attach to, Take off / Detach, the favourites and Remove from
  outfit act on every selected item, in one action; Edit, Touch, Object
  profile and Show original need a single item. Take off / Detach leaves
  body parts out and is named Remove (clothing), Detach (objects) or
  Take off / Detach (both). A selection mixing worn and unworn items
  offers Wear / Add for the unworn ones and Take off for the worn ones.
  The outfit editor keeps one selected row.
- `folders` / `folders-outfits`: the gallery or the list with outfit
  folders in My Outfits: "Beach" (one outfit and a "Night" subfolder with
  one), "Club" (two outfits) and an empty "Spring". The list shows them as
  collapsible rows (a click closes or opens one, in the gallery too), the
  gallery as sections ("Beach", "Beach › Night", "Club", "Spring", then
  "Other outfits"). Right-click an outfit for **Move to folder** (My
  Outfits, the folders, a new folder taking it at once; the folder holding
  it checked), a folder for New folder, Rename folder, Move to folder and
  Delete folder (greyed on a folder holding the worn outfit). The gear
  menu has New folder, Save as has **Save in**. In the default window:
  `AURORA_DEMO_POINTER="106,411,r"` (Club Black, list) opens the outfit
  menu, `"89,386,r"` the Club folder's, capture at frame 380;
- `folders-many` / `folders-many-outfits`: the same gallery/list with 320
  extra nested destination groups. Open **Move to folder**, scroll to
  **Wardrobe 079 / Style 319**, and move an outfit there. The submenu stays
  inside the viewport and **New folder** stays below the scrolling list.
  Repeat at a small window height or enlarged interface scale. In Inventory,
  drag a saved outfit or a group into another My Outfits group or back to
  My Outfits; protected folders, cycles, excessive depth and loose item drops
  must remain refused.
- `outfits`: the list of outfits; `worn`: the worn items. A right click on
  an outfit's header opens the outfit menu: `AURORA_DEMO_POINTER="82,372,r"`
  (Neurolab) or `"90,397,r"` (the worn outfit) in the default window, capture
  at frame 380;
- `save`: the Save as dialog; `edit`: the outfit editor;
- `menu`: an expanded outfit with an object that is not worn, for the right
  click menus;
- `duplicates`: replays folder answers with repeated links and
  originals coming from other parents.
- `broken`: the list with the third outfit expanded; it and the current
  outfit hold a "Deleted scarf" link whose original the server answered
  missing, as on the grid after the original was deleted. The scarf shows
  greyed, without an icon; Wear, Add and Take off on any outfit work, skip
  it, and the rewritten current outfit drops it (no "still loading").
- `take-off-folder`: the Worn tab, with an ordinary "Demo delivery"
  folder in the inventory (not a saved outfit) holding the worn jacket and
  pants and, in an "Accessories" subfolder, a link to the worn mesh hair.
  At frame 600 the inventory's Remove from current outfit takes it off
  (#1849): the jacket, pants and mesh hair leave the Worn tab, the body
  parts and the other attachments stay, and no "This outfit folder cannot
  be modified" message shows. Capture at frame 720 or later.
- `layers`: the outfit editor with three worn tattoos, listed furthest
  from the body first as in the Legacy Viewer: Lipstick, Freckles, Star.
  A tattoo row drags within the tattoos (a violet marker shows where it
  lands, another type refuses it); a selected one's up arrow moves it over
  the next, and there is no arrow past either end. `layers-drag` drags
  Lipstick under Star from frame 701 and drops it at frame 760 (the list
  then reads Freckles, Star, Lipstick); `layers-hold` holds it over its
  landing place, for a capture of the marker.

`wearables`, `wearables-outfits` and `wearables-edit` show all 17 system
wearable types in Worn, My Outfits and the editor respectively. Each type
has its own icon, shared with Inventory. Body parts and clothing precede
attachments regardless of their names or descriptions; the three tattoos
remain Lipstick, Freckles, Star in Worn and the editor. The editor's unworn
rows use the same type-specific icons.

These operations stay offline; creating new clothing and
new body parts is not available yet.

**`AURORA_DEMO_OUTFITS=<count>[,closed][,scroll][,locate]`** — with
`AURORA_DEMO_APPEARANCE`, adds `<count>` generated outfits (names like
"Beach Day 0012") with 20 to 60 links each to a pool of 480 generated
clothes, objects and body parts, a thumbnail id (never loaded offline) on
one in four. `closed` leaves the window closed; `scroll` turns the wheel
over the list or gallery of the default window from frame 700, down then up
every 300 frames; `locate` presses the locate button of the current outfit
banner at start: the gallery or list scrolls to the worn outfit, selects
it and flashes it for 1.6 s (the flash is gone by frame 620, the selection
stays). With `AURORA_PROFILE=1`, read `ui` of `perf summary`
(and `u_inventory` in `max:`): with 3,000 outfits it is 0.5 to 0.8 ms with
the window open or scrolled and 0.4 ms closed; it was 31 to 37 ms open and
2.5 ms closed before the wardrobe's links were walked only on inventory
changes and only the rows in sight laid out.

The demo objects use distinct points (skull, left ear,
left ring finger, left foot and top-right HUD), shown in the
lists of Appearance and Inventory.

**`AURORA_DEMO_REZ=1|land|object|contents|nocopy|cancel`** — opens the synthetic
inventory. `1` allows testing with the mouse; the other modes drag
a real inventory row from frame 680: terrain, opaque object,
contents of the object with Ctrl, no-copy item or cancellation with Esc.
The local simulator shows a synthetic cube at the rez point (no grid
asset). Take the screenshot after frame 700. Check that the no-copy item
disappears from the inventory, that Ctrl adds to the contents without a new cube
and that Esc creates nothing. Online, validate real objects, forbidden land,
the Trash / Library, neighbouring regions, the group of the land and
objects already worn; the human logs in and carries out these tests.

**`AURORA_DEMO_GESTURES=manager|editor|numpad|play|replay|channel|completion|completion-send|list|communicate|communicate-open`** — three synthetic gestures:
chat with a replacement and F8 shortcut, an animated pose on F9, and an
inactive chime on F10. `manager` opens the list; `editor` also opens the
chat gesture's editor; `play` starts the chat and animation at frame 650.
`numpad` opens the editor with the chat gesture bound to `6`: click the
world to leave text focus, then press the top-row and keypad `6` to compare
playback. Change and save its key or modifier to check other digits,
Ctrl and Shift; keypad digit shortcuts also work with NumLock off.
Capture at frame 620 for the list/editor, 680 during playback and 800
after the explicit pause. Sounds stay muted unless `AURORA_DEMO_SOUND=1`.
`replay` adds a clap gesture on F7 (the chime three times, 0.3 s apart),
a gesture on Ctrl+Z and one on Ctrl+B, and opens the Ctrl+B gesture's
editor, whose shortcut row warns that the build tools keep Ctrl+B. The
scenario presses F7 at frames 650, 665 and 680, then Ctrl+Z at 760 and
Ctrl+B at 780, through the rules of a real key press: run it with
`AURORA_FPS_LIMIT=60` and read the `demo gesture key` and `demo sound
trigger` lines of the log (each F7 press starts the clap over; Ctrl+Z
plays its gesture while the build tools are closed; Ctrl+B is kept by
them). Capture frame 620 for the editor's warning.
`channel` adds an active `/hudping` gesture whose chat steps command worn
scripts: `/-4242 ping`, `/5 test` and `/5test`, and sends `/hudping` from the
chat bar at frame 650. Script-channel chat is never echoed as local chat:
the demo's worn listener answers each line ("Heard on channel -4242: ping"),
and the log has one `demo channel chat: channel=…` line per step. Capture
frame 800 (`AURORA_FPS_LIMIT=60`) to see the three answers. On the grid, a
negative channel goes as a `ScriptDialogReply`, as in the Legacy Viewer.
`completion` gives the chat gesture the `/eject` trigger, focuses nearby
chat at frame 540 and types `/ej` at frame 550: capture frame 620 to see
`/eject` with the added `ect` selected. `completion-send` also presses
Enter at frame 650: capture frame 800 to see the gesture's chat steps and
the cleared input. Typing replaces the selected suffix; Backspace removes
it without immediately completing it again. Only active, enabled gestures
participate; ambiguous prefixes extend only to their common suffix. The
same completion is available in the conversations window's local chat,
with instant messages and numeric channel prefixes keeping their input.
`list` (#980) adds an active `/hug` gesture, leaves the manager closed,
focuses nearby chat at frame 540 and types `/h` at frame 550: the list
above the bar shows `/hello` (with what it says) and `/hug`, none
highlighted since they share no further letter; Down at frame 600 writes
`/hello` with `ello` selected and highlights it. Capture frames 580 and
650. Tab keeps the trigger and closes the list, Enter says it, Escape
closes the list; Preferences › Chat › "Complete gestures in nearby chat"
turns both the list and the inline completion off.
`communicate` starts with the gesture manager closed, clicks the wolf menu
at frame 540 and hovers Communicate at frame 560: capture frame 620 to see
the Gestures entry below Conversations, with Ctrl+G in the shortcut column.
`communicate-open` also
clicks that entry at frame 650: capture frame 800 to check that the manager
opens through the actual menu control.
Check Ctrl+G, filtering and active-only/enabled switches, sorting and
multiple selection, inventory Open/Play/Activate/Deactivate, all four step
types, stock/inventory animation choices, Shift/Ctrl and key choices,
preview/stop, reordering, saving and discard confirmation. Chat steps keep
their channel prefixes (`/28574 T4`); the demo never connects to a grid.
Online validation is performed by the human: activate and play existing
gestures, chat/key triggers on the real keyboard, save/reopen modified
gestures, permissions, outfit switching, teleport and restoration after
relogging. A rejected save must keep the draft, and deactivation must stop
only the gesture's requested motions.

**Inventory menu customization:** saving/restoring a menu notecard reports success in theme green, clears older warning text and disappears after five seconds in both inventory window presentations. Failed callbacks remain warnings and do not inherit an older green notice. Restoration wording omits Shared/common. `aurora-menu-restored` simulates a native notecard callback: capture before five seconds after the loading overlay fades (for example 240 frames at 60 FPS) for the green confirmation, and after five seconds (420 frames) for its disappearance. Locked menus hide empty vertical, horizontal and icon groups after live commands and saved placements are resolved, including parents containing only empty groups, bars or separators. Disabled commands and the movable Lock remain valid content. This is presentation only: stored groups, placements and absolute cells survive and reappear in customization or when applicable commands return. Filter queries still show their no-results state on nonempty groups so users can clear them. Use `aurora-menu-empty|aurora-menu-empty-custom` for the same emptied Attachments group and unused custom group in locked/editor modes. While unlocked, every inventory command is available for arrangement regardless of the original right-click target. Inapplicable options are greyed but draggable; their native command bindings remain absent. This includes Trash commands, folder commands, animation/sound/environment actions, worn actions and body/HUD points. Locked menus still resolve only the applicable native commands. Use `aurora-menu-all-texture|aurora-menu-all-folder|aurora-menu-all-trash` to inspect the same complete editor from three different sources, and `aurora-menu-texture|aurora-menu-trash` for the corresponding locked menus. Move an animation action from the texture editor into an icon bar or hidden storage, save/reload, then verify its placement on an animation without adding it to the locked texture menu. Existing hidden options remain in the customization companion. Use `AURORA_DEMO_INVENTORY=aurora-menu-object|aurora-menu-folder|aurora-menu-animation|aurora-menu-script|aurora-menu-link` for the common locked arrangement resolved against the native commands of each type, `aurora-menu-organize` for the expanded Organize group, and `aurora-menu-custom|aurora-menu-locked` for a saved arrangement with empty icon cells, text Copy, a custom group and horizontal Organize. `aurora-menu-hidden` opens the editing companion with hidden Image and Organize entries. The movable Lock option opens/closes the adjacent settings panel (Hidden options, Create a group, Reset menu); the default bottom icon bar places Lock in cell 1 and native Delete in cell 8. Drag Lock into another icon bar or into text to display the localized Lock label (Verrouiller in French); it still toggles customization while ordinary commands stay blocked during editing. Lock and any containing group/bar cannot be placed in Hidden options, including by moving an ancestor or by right-click. Saved layouts that hide Lock recover it as a visible text option; old fixed Delete-footer placements migrate into the bottom bar. Unlocked menus use a grab cursor and suppress command execution, including deletion: only layout editing and group navigation work. Drag text, icons, groups and dividers into Hidden options and back; hidden descendants retain their permissions and are absent when locked. Right-click also offers Hide/Restore. Compare the customizer with Construction `general+organize`: shared typography, violet Hidden options heading with close action, inset storage, full-width actions and the pointer-following drag preview. Its width resets for each command so a short previous title cannot force the next title into a narrow column. Losing window focus, clicking outside the menu and its editing panels, or dismissing the owner automatically relocks, cancels an unfinished drag/name editor and preserves placements; another right click must open a usable locked menu. Moving between the menu, settings and child panels keeps editing active. Close or Finish organizing relocks without losing placements; the companion follows the menu on the free side with aligned bottoms. Every icon bar reserves eight absolute cells. While editing, its drag grip sits to the left and its removal trash to the right on the same row; no title row is shown. Add an icon bar in settings; its organizing-only left grip drags the whole bar and its right trash action (also offered on right click) removes it, restoring its members as text in visible icon order, including temporarily unavailable commands. The two default bars can also move, hide or be removed; reset restores them. Newly added unused bars remain available. Once populated, a bar disappears automatically when its last option is moved out (including into Hidden options or a new group); undo restores it. Temporarily unavailable commands retain their saved bar and cell across restart. This also applies to the default bars and bars inside groups; empty parent groups follow their usual cleanup rules. Use `aurora-menu-bars` for a filled added bar with gaps, an extra empty bar and the default bottom bar removed. `aurora-menu-drop` holds a synthetic layout drag, starting with Image and switching to Attach to HUD after 60 pointer frames. Set AURORA_DEMO_POINTER to the top edge, center or lower edge of a group header to capture the before/inside/after indicators: an upper line inserts before, an outline puts the option inside, and a lower line inserts after the entire expanded group. Edge insertion keeps closed groups closed; only the center expands them during dragging. This capture fixture keeps synthetic window focus while recording so the real focus-loss behavior does not dismiss its menus in the background. Icon groups (New, clothing categories or a group moved into a bar) open on click, never on hover; a second click closes them. Text submenus retain hover navigation, and editing still blocks native commands. Repeating a stationary right click dismisses the latest editing panel, then the root menu. Layout, hidden placements, enabled group filters, expansion and lock state live in global settings as one arrangement across all inventory types and accounts. Legacy per-type layouts migrate once, preferring an edited object layout, then edited folders or another edited type. Consecutive dividers render once; leading/trailing dividers disappear in every group after applicable commands and search results are resolved, without moving their stored identities. Right-click a group while organizing to add/remove its filter. The field works in locked vertical, horizontal and icon groups; matching descendants retain their parent path, clear restores the list, and Lock remains reachable. Queries are temporary and clear on dismissal. `aurora-menu-filter` shows a vertical Attach to group with synthetic French input `main`. The settings companion offers Save to a notecard and Reload from a notecard. Save captures the common arrangement, groups, dividers, absolute icon cells and enabled filters; load validates the entire bounded/versioned inventory-menu document before applying it and relocks the result. Construction profiles and malformed/incompatible files are rejected without changing the current menu. Native upload request IDs and downloaded item IDs correlate callbacks; live notecard type/copy permission, Trash ancestry and preview restrictions are rechecked. `aurora-menu-export` and `aurora-menu-load` show the native save/picker dialogs; `aurora-menu-load-search` shows a matching notecard with its parent folders. Reload closes its owner context menu and opens a texture-style picker with a search field and non-empty inventory folders. Later text-field menus remain usable. Browse nested folders or type a notecard name: results keep their real parent path, and clearing search restores normal browsing expansion. Other asset types and Trash contents are excluded; non-copyable notecards cannot be selected. Selection alone issues no download; Reload rechecks the live item and preview permissions. Reset restores the supplied default arrangement: Image and Move to lead Organize, asset UUID is at the root before Organize, and Attachments/Organize start expanded. Existing saved layouts are retained until explicitly reset. Human grid validation: save a named notecard, alter the common menu, reload that notecard, and confirm all item types recover the arrangement and group filters while native permissions still apply. Native offline input tests cover drops, visibility, editing isolation and the lock; the human validates real inventory operations and persistence across accounts online.

**Expanded inventory layout (#1020):** `aurora-columns` opens the wide explorer with a resized folder sidebar and custom column widths; `aurora-columns-hidden` keeps only Name and Permissions. Compare `aurora-medium`, `aurora-wide`, `aurora-rights` and the compact `aurora` layout. Drag the sidebar gap or column-header boundaries, hold the pointer stationary, then resize/reopen the window: widths remain saved and rows stay aligned with their headers. Double-click a boundary to restore its default widths. The Columns toolbar menu toggles Type, Permissions and Modified independently; Name stays visible. Reset inventory layout restores all columns and responsive widths. Check folder windows, filtered results, ordinary row selection and drag/drop at 100% and 150% UI scale. Medium views now give permissions their own column instead of appending them to the name; compact trees keep their existing inline rights. These controls are local and need no grid connection.

**`AURORA_DEMO_INVENTORY`** — opens a synthetic inventory: personal
folder, subfolder, outfits, objects, documents, texture, sound, gesture,
sky, material, link and varied permissions; library in the tree,
root open and labels aligned left. Creations and mutations simulated
offline; the files and snapshots uploaded in demo stay offline.

- `1`: the inventory alone. This mode and every one below but the
  `aurora` ones were written for the Legacy Viewer's window and show it,
  whatever Preferences › Aurora › Inventory says.
- `landmark`, `aurora-landmark`: a landmark's context menu in each inventory
  layout. Capture at frame 700. Copy SLurl must copy the destination's actual
  region and coordinates; Show on map must open and center the map without
  teleporting. Both actions resolve an initially unloaded landmark offline.
- `uuid`, `uuid-restricted`, `uuid-missing`, `uuid-multi`: the texture's
  context menu with Copy asset UUID enabled, disabled by a missing transfer
  permission, disabled by an unavailable UUID, or disabled by multiple
  selection. Capture at frame 700. In `uuid`, click the entry and paste into
  a text field: it must be the asset UUID, not the inventory item's UUID.
  Compare `object`, `script` and `link`: the entry must be absent because
  these types never expose an asset UUID. These visibility rules apply to
  both inventory layouts and folder windows through their shared menu.
- `show-original`, `aurora-show-original`: minimize the inventory at frame
  650, then run an attachment's "Show in inventory" action at frame 750.
  Capture frames `700,850`: first the title bar alone, then the restored
  window with the demo object selected and revealed. Covers both layouts
  without grid access; also test manually with a worn attachment on Agni.
- `outfit-menu`, `outfit-menu-loading`: a saved outfit's inventory context
  menu with Appearance closed. The first has its originals loaded and the
  replacement/addition entries enabled; the second withholds one linked
  original and keeps them disabled while the menu asks for it. The demo
  does not answer that network request. Capture at frame 700. On a grid,
  log in without opening Appearance and replace an outfit from Inventory;
  after its contents arrive, the entry must activate in the same open menu.
- `favorites`, `aurora-favorites`: the Favorites tab with three objects of
  the same type, added through the batch planner. `favorites-multi` and
  `aurora-favorites-multi` show their selected rows and Add to Favorites
  menu; click it to check that every object receives its own star. Capture
  at frame 700; the `aurora` variants exercise the Aurora layout.
- `aurora-sidebar-medium`, `aurora-sidebar-wide`: the expanded inventory
  with 80 extra folders for #970. Click folders near the top and bottom
  of the left column, then scroll it and repeat, including partly visible
  rows: the clicked folder must
  stay under the pointer while the explorer lists its contents. Capture
  at frame 700. Also use Find original from an inventory link to check
  that navigating to an offscreen folder still brings it into sight.
- `aurora`, `aurora-medium`, `aurora-wide`, `aurora-rights`: Aurora's
  window, placed at frame 2 at its narrow (440 px), medium (800 px) and wide
  (1180 px) width, on the demo object of the right-click tests, the demo
  shirt in the favourites; `aurora-rights` is the wide window with the
  CMT menu on "Both". Capture at frame 700 with `AURORA_FPS_LIMIT=60`.
  At every width, the search field and advanced-search button occupy a full
  row below the tools. Collapse all / Expand all are directly accessible in
  the toolbar: they fold the compact tree or filtered results, and the folder
  sidebar in the explorer. Expand, collapse, then manually reopen a parent:
  its descendants must remain closed. Check a folder window and hidden Library
  too; commands stay within that window's scope and retain the current search.
  Narrow: icon tabs, toolbar (filters, CMT, Sort, New, Collapse all, Expand all), the tree with
  C/M/T at the end of each item, the selection's card (Wear, Add,
  Properties, More). Medium: the column of shortcuts (with their counts) and
  folders, the explorer of the revealed folder with Type and Modified.
  Wide: breadcrumb, Ctrl F inside the empty search field, the filter count,
  the details pane (picture, Give, Properties, creator, owner, acquired,
  next owner switches) and the status bar. Hover an object or clothing row
  for Add / Take off (and More at the wide width); Add preserves existing
  attachments and clothing regardless of the double-click preferences.
  Body parts and physics still offer Wear because they cannot stack. Widen the narrow
  window with the title bar button.
- `aurora-no-details`, `aurora-medium-no-details`,
  `aurora-wide-no-details`: the same three widths with the selection's
  details hidden (the bottom-bar button of the toolbar at the narrow and
  medium widths, the right-pane button at the wide one, unlit): the list
  takes the room of the card or of the pane. Click the button to show them
  again; the choice is kept with the inventory preferences, so a later
  normal launch keeps it.
- `aurora-folder-window`, `aurora-folder-window-wide`: the same, with the
  right-click tests opened in a folder window ("Show in a new window") beside
  the narrow main window, narrow (420 px) or wide (1180 px). No tabs; the
  column holds the folder itself and its subfolders only (no shortcuts, no
  Trash); the explorer and the breadcrumb stop at the folder; a search stays
  inside it.
- `aurora-additional`: the narrow main window, its "Additional inventory
  window" button (a second briefcase, right of CMT) lit, and the additional
  window on its left: the whole inventory with its tabs but no Received
  tab, and no such button. Click the lit button: the additional window
  closes; click it again: it reopens at the same place, its folders and
  search forgotten. Drag an item from one window onto a folder of the other.
- `smart`, `aurora-smart`, `aurora-smart-wide`: advanced search. The window
  is in advanced mode on a saved smart search: objects or scripts, or a name
  ending in a number (an Any group), but not the no-copy object (a Not
  rule). `aurora-smart` shows the narrow window, whose chip bar keeps its
  chips and its field only; `aurora-smart-wide` shows the whole chip bar
  (All/Any, add, save, leave) and the "Smart searches" sidebar section with
  two saved searches, the first highlighted; `smart` shows the classic
  window with the rule summary and the rule builder open in the Filters
  floater. Click a chip to change its operator, Not or remove it; flip
  All/Any; right-click an item for "Find similar"; the ✕ of the chip bar (or
  "Quick search" in the search options) brings quick search back with its
  text. The demo keeps saved searches in memory only.
- `aurora-worn`, `aurora-worn-medium`, `aurora-worn-wide`: Aurora's three
  widths on the clothing folder with no selection. Worn clothing names stay
  semi-bold and use the exact text colour of their Worn tag beside the
  regular spare shirt, without needing a click. Change the theme or its
  colours: worn names and their tags must stay the same colour. Compare an
  embedded font with `AURORA_UI_FONT=system:C:\Windows\Fonts\segoeui.ttf`
  (the standard semi-bold face is used without synthetic overpainting).
  Capture at frame 700; removing clothing must restore its regular weight
  and text colour on the next frame.
- `permissions`: expanded inventory with the approved additive next-owner
  indicators enabled. Combine with `AURORA_DEMO_BUILD=contents-ready,object:118`
  and `AURORA_DEMO_BUILD_TAB=contents+next-owner` to inspect both windows.
  Each switch independently hides/shows only the suffix after the arrow;
  current rights and the detailed hover remain available. Check filtered,
  folder and unknown-link rows, horizontal scrolling and item selection.
- `delete-key`, `delete-button`: select a folder containing three
  levels and their items, then open the confirmation with the Delete key or
  the trash button of the window footer. Capture 660 (selection and trash to
  the right of the counter), then 720 (confirmation identical to the context
  menu). Delete moves the whole folder to the Trash; Cancel keeps it in place.
  Also check a multiple selection, a worn / protected object (command
  unavailable), a text field (the Delete key edits the text) and a folder
  window. A selection entirely in the Trash asks for confirmation
  before permanent deletion.
- `keyboard`, `aurora-keyboard`, `aurora-keyboard-wide`: click the demo
  object at frame 650, extend its selection with Shift+Down twice, then
  shrink it with Shift+Up at frame 710. Capture at 690 (three selected rows)
  and 720 (two), in the Legacy, compact Aurora and wide Aurora layouts.
  The log reports the selection count and visibility at both capture points.
  Manually check held arrows, direction reversal, open-folder boundaries,
  filtered lists, folder windows and arrow editing in search/rename fields.
- `keyboard-fold`, `aurora-keyboard-fold`: click the demo object at frame
  650, press Left at 665 (its folder is selected) and 680 (that folder
  closes), then Right at 700 (it opens again). Capture at 690 (the folder
  closed and selected) and 720 (open). The log reports the selection and
  the folder's state at both points.
- `clipboard`: expanded inventory to test Ctrl+C / Ctrl+X / Ctrl+V on
  items, folders and multiple selections. Choose the destination
  folder (or one of its items) before pasting. The search and
  rename fields keep their text shortcuts.
  `clipboard-copy` selects two items, copies then pastes twice into
  the subfolder; `clipboard-cut` moves two objects, one of them no-copy
  and one no-transfer; `clipboard-folder` copies the subfolder, its two
  nested levels and their three items into "Test destination".
  `clipboard-cut-folder` moves the same tree, with a no-copy object
  in a subfolder: all the items must stay present with the
  same identifiers. `clipboard-window` copies from a folder
  window then pastes into another, empty one. Gestures at frames 650–781, then
  opening of the destination folder; screenshot after 800. No activation
  of the native window and no reading of the system clipboard during these
  automatic scenarios.
  `clipboard-progress` illustrates the possible states of the status line:
  update at frames 650–749, copy with a 2/3 counter at frames
  800–849, verification at frames 850–919, then end confirmation from 920.
  Capture 700,820,890,940; the animation, the counter and the elapsed time must
  stay visible in the status line, without enlarging the window.
  The verification concerns metadata mutations; a real copy is finished
  by the reception of its items, without a final re-read of the folder.
  Online, the status must show the steps, the counter of confirmed items and
  the elapsed time, then "Update completed". The items appear as soon as
  they arrive; it must be possible to start another copy or move
  another item while a copy is in progress. A cut/paste changes the tree
  as soon as it is sent reliably and shows "Move requested" without waiting for
  an HTTP re-read. Test two successive copies, folders with several levels,
  then reload and check the originals, the copies and the moves.
  An unconfirmed copy asks to reload the destination; incomplete
  references ask to reload the source instead of waiting for a timeout.
- `move-menu`: menu of an item with "Move to…"; `move-item`,
  `move-folder`, `move-multi`: destination window for an item, a
  folder or three objects (one of them no-copy and one no-transfer).
  `move-filter`: filter on the subfolder; `move-window`: same window
  from an inventory limited to a folder. Select a folder then
  Move; Cancel must leave the selection in its initial location.
- `offer`, `offer-reject`: object offered by a resident, kept or rejected
  (see `AURORA_DEMO_NOTIF` above). `offer-card`, `offer-card-keep`,
  `offer-card-legacy`, `offer-card-legacy-reject`: its card in the giver's
  conversation, kept from the card or rejected from the notification.
- `folder`, `object`, `animation`, `script`, `note`, `properties`, `nocopy`,
  `notransfer`, `link`: the menu of the chosen item, or its properties.
  `clothes`, `body`, `settings`, `uploads` are aliases of `folder`, to be
  combined with `AURORA_DEMO_POINTER` to hover the submenus.
- `received`, `received-filtered`, `received-missing`: Received purchases tab,
  search limited to its Marketplace box and state with no received folder. The
  list and the Collapse / Expand buttons keep the same presentation; no
  personal folder must appear in this tab.
- `received-sort`, `received-sort-name`, `aurora-received-sort`,
  `aurora-received-sort-name`: open Received preferences in the classic or wide
  Aurora layout. Three purchases contain files dated one, two and three hours
  ago. With newest first and "Folders always by name" off, folders show Z, M, A;
  checking it shows A, M, Z. The `-name` variants open the checked state for
  comparison captures. With the general "By name" order, both checkbox states
  show A, M, Z. Main inventory sorting must stay independent, including after
  saving/reloading preferences; Interface section reset restores both defaults.
- `received-big`, `received-big-open`: a long-time buyer's Received
  purchases (150 purchases of 20 items). The purchases arrive closed, open
  and close one by one, and only the rows in sight are drawn; `-open`
  expands them all. With `AURORA_PROFILE=1`, `ui` stays under 1 ms (it was
  about 17 ms, all open, before the tab used the result tree).
- `big[:<items>]`: a long-time resident's inventory, 30,000 items by
  default in about a tenth as many folders, five levels deep (store
  brands, products, extras, fit packs) and a loose folder of 2,000 objects,
  open with the first top folder, three of its brands and their products.
  Options after commas: `recent` (Recent tab), `search=<text>`,
  `toggle=<s>` (opens and closes the window every `s` seconds, to compare
  closed and open in one run; each change is logged as
  `demo inventory window open=…`), `churn=<ms>` (one item changes every
  `ms` milliseconds, like the server's inventory updates) and `replace`
  (opens Replace links on the first object, which a link of every fifty
  items points to: with `big:300000`, about 120,000 objects to choose
  from; the frame rate must stay that of the window closed). With
  `AURORA_PROFILE=1` and `AURORA_FPS_LIMIT=120`, read `u_inventory`: in the
  dev build, the window open costs about 0.4 ms per frame (12.9 ms before
  only the rows in sight were laid out) and no slow frame when it opens; an
  update costs one frame of about 6 ms.
- `animation-open`, `animation-properties`, `image`, `image-photo`,
  `image-picker`: the five reference panels.
- `animation-play-local`, `animation-play-world`: open an animation through
  the ordinary inventory action, then start local or public playback. Exactly
  one animation preview must remain, with Stop in the selected mode and the
  other mode disabled. Stop and replay must keep that same window (#1445).
  `animation-open` uses the same opening path without starting playback, for
  checking both buttons manually. Capture around frame 700.
- `note-open`, `note-new`, `note-readonly`: editor of an existing note,
  empty note without an asset or note that cannot be modified. Edit the text
  then Save (or Ctrl+S); closing and reopening keeps the text during the
  demo session. Closing with changes offers Save,
  Discard changes or Cancel; Save only closes after
  the save is confirmed.
- Notes open by default at 760 × 600 pixels (adapted to the viewport);
  the text must fill the available height, with Save (Ctrl+S),
  the counter and the message, if any, just below. Also check after
  resizing and after collapsing / restoring the window.
  The item description is visible above the view tabs. Change it and
  press Enter or click outside the field: only the metadata is saved,
  leaving any content draft unchanged. Check the same field in Classic,
  Markdown and Edit, in object contents, and disabled on a read-only item.
- `note-task`, `note-task-readonly`, `note-task-save`, `note-task-error`:
  same editor for a note inside an object. The last two add
  text at frame 650 then send Ctrl+S at frame 670; the refusal keeps
  the draft. Saving and closing / reopening keeps the text during
  the demo session, without creating an item in the personal inventory.
- `note-classic`, `note-markdown`, `note-source`: the same Markdown sample
  with an image, a landmark and an object in each display mode. Capture
  after frame 740. Switch tabs, edit the source in Classic or Edit, then
  save from Markdown and reopen: the Markdown syntax, Unicode text and
  attachments must survive. Classic defaults to Firestorm's plain,
  proportional, wrapping text; Edit shows the monospace source on the left
  and its live Markdown preview on the right, with independent scrolling.
  Type headings, bold and Unicode in Edit and check that the right side
  updates without leaving the mode, then Ctrl+S from either pane.
  The formatting toolbar applies bold, italic, strikethrough, inline code,
  links, headings, lists, quotes and fenced code to the source selection.
  Select Unicode text with the mouse, click Bold, then undo and redo:
  the text, selection and attachments must survive, and Ctrl+S must save
  the Markdown source. Repeat a style to remove it; without a selection,
  the controls insert editable templates. Markdown help shows syntax and
  limits. Formatting is disabled for read-only notes and pending saves.
  `note-format` uses the same sample in Edit, selects the first list word
  at frame 650, clicks Bold at 670–671 and opens Markdown help at 710–711;
  capture frame 700 for the selection/preview or frame 800 for the help.
  Leave Edit idle with the source focused, including at a fractional UI
  scale (112.5%): neither pane's border may oscillate. A short note must
  not show scroll bars; a long note must still scroll in each pane.
  Markdown is a selectable formatted preview with headings, emphasis, lists, quotes,
  code and links. Images show their alt text as a link without downloading.
  Newlines before and after embedded-item rows must remain newlines in both
  Markdown views; items written on the same source line must stay inline.
  `note-remember` opens this same sample using the saved preference instead
  of forcing a view. With no saved choice, Classic is selected. Choose
  Markdown, open another inventory or object note, then restart the demo:
  newly opened notes must still use Markdown. Repeat with Classic and Edit.
  Merely displaying an older window must not replace the last explicit
  choice, and switching views must preserve any unsaved source/attachments.
  Demo settings stay separate from the normal viewer's settings.
  `note-lsl` / `note-lsl-markdown` show fenced LSL in Edit / Markdown. Capture
  after frame 740: types, functions, events, constants, strings, numbers and
  comments must use the script editor's colors, including customized colors.
  The LSL toolbar button wraps selected lines or inserts an editable script
  template. Type Tab/Shift+Tab with and without multiline selections; in LSL,
  Enter keeps indentation, a closing brace aligns with its opener, Backspace
  removes a four-column stop and Ctrl+K / Ctrl+Shift+K toggle comments. Check
  undo/redo, Unicode, exact paste, live preview and Ctrl+S with attachments.
  Prose/other languages must not gain LSL colors or brace/comment behavior;
  Classic and read-only notes must retain their existing behavior.
  Fenced/indented code must have one continuous background across the block's
  full width, including blank and wrapped rows. Repeat in both previews at a
  narrow window width; inline code keeps its small highlight, selection stays
  visible and neither scrolling nor switching views changes the saved source.
- `note-edit`, `note-save`, `note-close`, `note-error`, `note-embedded`:
  automatic addition of text at frame 650, then save with Ctrl+S
  (`note-save`), close confirmation (`note-close`) or simulated refusal
  (`note-error`) at frame 670. `note-embedded` contains an image, a landmark
  and an object: their icons and names are shown in the text; attachments
  still referenced are kept when saving. Screenshot after
  frame 720; the refusal must keep the draft and allow a new
  attempt. A left click opens the item directly; the right click
  offers Open and Copy to inventory. An image shows its preview,
  a landmark opens its place, a sound plays locally; the other types offer
  a copy into the inventory.
  Dragging an inventory item into the text inserts it at that spot
  if the next owner has copy / modify / transfer.
- `note-drop`: drops a texture at frame 651 then saves at frame 670.
  `note-menu-right`: opens the menu on the texture by right click at frame 651.
  `note-image`, `note-landmark`, `note-copy`: opens respectively
  the image, the landmark or the object with a single left click at frame 651;
  `note-copy` confirms the copy at frame 671. `note-task-embedded` makes it possible
  to test the same attachments in a note inside an object. Take the screenshot
  after frame 740. Any added item must be saved before it is opened.
  Landmarks are fetched into the inventory before their place is opened,
  as in Firestorm; a landmark already present for the same asset is reused.
- `image-photo-save`: captures then saves a thumbnail at frame 2100
  and closes the snapshot window while keeping the Image editor open.
  Capture at frame 2250 to inspect the saved message in both the editor and
  inventory footer, then at frame 2700 to see only the editor's result.
  Network mock tests cover stale and unreadable AIS snapshots followed by the
  expected thumbnail UUID, permanent mismatches and bounded failure: one write,
  read-only retries, for both items and folders. UI tests keep loading visible
  until completion and expire the footer after five seconds on success or error.
  On the grid, the human checks that uploading stays on Loading until confirmed,
  then shows Picture saved; an actual failure remains visible in the editor.
- `image-layout` / `image-layout-wide`: open the local-file thumbnail choices
  for an item with a synthetic portrait / landscape image. Add `-folder`
  before `-wide` (`image-layout-folder`, `image-layout-folder-wide`) to target
  a folder instead. Click a preview to use the whole image stretched to the
  square, fit it proportionally with transparent margins, or crop it.
  Drag the image inside the right-hand preview to move the crop up/down for a
  portrait or left/right for a landscape. The image follows the pointer and
  stops at its edges. Releasing the drag does not upload; click the adjusted
  preview to use it. Check both extremes, a partial move, repeated drags and
  square/small images, on both an item and a folder. The saved thumbnail must
  contain the selected area rather than returning to the centre.
  No upload occurs before choosing; Cancel or Escape keeps the old image.
  Repeat using Upload from your computer with a PNG, JPEG or JPEG2000 file;
  verify transparent pixels, square images and all four source edges. Scene
  snapshots retain their square viewfinder. The human checks persistence
  after reconnect on real inventory items and folders.
- `additional`: the Legacy Viewer's window with its "Additional inventory
  window" button lit at the bottom, right of "+", and the additional window
  opened on its left: Inventory, Recent, Worn and Favorites tabs but no
  Received, and no such button at its bottom. The button closes it and
  opens it again.
- `folder-window`: a window limited to the contents of the personal folder,
  with its name as the title and a filter of its own; `folder-window-search`
  shows this search limited to the descendants.
  Check the compact C·M·T indicators: granted rights green, denied
  rights red and struck through (no-copy object, no-transfer object), detail
  of the current owner and of the next owner on hover. A link
  takes the permissions of its original.
- `rename`, `new-script`, `new-note`, `new-folder`: renaming in the
  row, without a window (their screenshots simulate the focus of the field
  without activating the Windows window).
- `multi-add`, `multi-detach`, `delete`: the multiple selection and the
  delete confirmation.
- `sort`, `filters`, `preferences`, `recent`, `worn`, `filtered`, `large`:
  system / date sort, filters, preferences, Recent / Worn tree,
  filter excluding objects or search of 1,500 items.
- `long`, `long-filtered`: long names and horizontal scrolling in
  the full or filtered tree.
- `resize-left`, `resize-right`: with a screenshot, drag the corresponding
  edge beyond the minimum size then release the mouse.

Synthetic data only; no rendering benchmark.

The indicators are also visible in the contents of an object:
`AURORA_DEMO_ACTIONS=open` for the Open window, or
`AURORA_DEMO_BUILD="move,object:100"` with
`AURORA_DEMO_BUILD_TAB=contents` for the build tab. The demo
contents offer full rights, no-modify or no-copy;
the next owner only has transfer. Also check the double-click
on a note and the "no copy" search in the Contents tab. Hover and
select an item: same row, same icons and permissions after the name as
in the user inventory; the context menu and the opening of documents
stay available. Long names scroll horizontally in the list.

To validate notecards on the grid, open an existing note then a
new note, edit their text, save and close / reopen. Read them
again after logging out / logging back in to check persistence. Also test
a note with an attachment: it must stay present after the text is
edited. The re-read uses the inventory transfer of the simulator, like
Firestorm; protected notes and scripts are not read via ViewerAsset.
In a modifiable object, Contents tab of the build window (or Open
window of the object), double-click a note then use Save
or Ctrl+S. Right click › Open must give the same result; Rename
stays a separate action of the menu. Close / reopen, then do it again
after reconnecting. With a note or an object that cannot be modified, the
text must be viewable when copying is permitted and stay read-only.

What the inventory must show with these scenarios:

- **Filters** selects the types, permissions, links, creator and
  age; **Preferences** sets the sort, the tabs, the folders
  included in the search and the double-click. These options are also in
  Preferences › Interface.
- System folders come first; items are sorted from the
  most recent to the oldest and folders by name.
- The search can cover the name, the description, the creator or
  the UUID; `+` combines terms, `"word"` searches for an exact word.
- Items stay in their folders, including in Recent, Worn,
  searches and filtered views. Collapse / Expand acts on these
  folders; the field selector is to the right of the input.
- "Also show folders without matching items" adds the empty folders to
  the view, the parent folders of the results always being present.
- The list also scrolls horizontally when a name exceeds the available
  width; long names do not block shrinking the window.
  The controls and tabs adapt to small widths. At the minimum
  size, the dragged edge stops without moving the opposite edge of the
  window.
- The path and the date stay available in a tooltip. Recent starts from the
  last logout (24 h on first launch).
- Filters stay specific to each window; "Keep by default"
  remembers them for the next openings.

## Media and sound

| Variable | Effect |
|---|---|
| `AURORA_DEMO_MEDIA=<url>` or `1` | Media on a prim |
| `AURORA_DEMO_PARCEL_MEDIA=<url>` or `1` | Parcel media |
| `AURORA_DEMO_MEDIA_CLICK="x,y[,x2,y2]"` | Clicks on the media |
| `AURORA_DEMO_MEDIA_CLICK_FRAME=<n>` | Starting frame of these clicks |
| `AURORA_DEMO_MEDIA_PACK=downloading`, `installing` or `failed` | Fixed state of the media plugins download |
| `AURORA_MEDIA_PACK_FEED=http://127.0.0.1:<port>/` | Media plugins pack taken from a local test server |
| `AURORA_DEMO_MUSIC=<url>` or `1` | Music of the demo parcel |
| `AURORA_DEMO_STREAM_TITLE=[window\|history\|text][,stop]` | Stream title window with changing titles and their history |
| `AURORA_DEMO_MEDIA_FILTER=ask`, `first`, `music`, `list`, `worn` or `hud` | Media filter question or list; media on our own worn items |
| `AURORA_DEMO_SOUND=1`, `hud` or `parcel-open\|parcel-local\|parcel-source` | World/HUD chime or parcel sound isolation; interface and media sounds audible |
| `AURORA_DEMO_SOUND_OCCLUSION=1` | Three looping sounds: in the open, behind a wall, shut in a box; each one's occlusion in the log |
| `AURORA_SOUND_OCCLUSION=0` or `1` | "Muffle sounds behind walls" forced off or on for this launch, without saving it |
| `AURORA_DEMO_SOUND_HRTF=1`, `circle` or `updown` | A sound circling the listener and one rising above it and falling, for the 3D sound for headphones |
| `AURORA_SOUND_HRTF=0` or `1` | "3D sound for headphones (HRTF)" forced off or on for this launch, without saving it |
| `AURORA_DEMO_SOUND_REVERB=1`, `room` or `hall` | A clapper above our avatar, a small room and a large hall to walk into (or to start in), for the reverb of the surroundings; its estimate in the log |
| `AURORA_SOUND_REVERB=0` or `1` | "Reverb from the surroundings" forced off or on for this launch, without saving it |
| `AURORA_DEMO_MINIMIZED_MUTE=1\|off` | Force mute when minimized on or off for an offline run |
| `AURORA_DEMO_SOUND_EXPLORER=1\|all\|blacklist\|blacklist-ignored\|avatar-menu\|avatar-menu-mixed\|avatar-menu-pie` | Sound Explorer, separate blacklist categories, ignored objects or avatar sound-category switches |
| `AURORA_DEMO_ANIMATION_EXPLORER=1\|blacklist` | Animation explorer with a stand-in preview, or a blacklisted animation |
| `AURORA_DEMO_WIND=<x>,<y>[,fly]` | Uniform region wind for the wind sound; `fly`: arrive flying |
| `AURORA_DEMO_VOLUME=open` or `muted` | Volume panel of the top bar pinned open |
| `AURORA_PERCEPTUAL_VOLUME=0` or `1` | The volume sliders' perceptual curve forced off or on for this launch |
| `AURORA_DEMO_VOICE=speaking`, `loud`, `listening`, `off`, `cycle` or `sweep` | Voice states and colours of the avatar name tags; `sweep`: a plate from a whisper to a shout |
| `AURORA_DEMO_CALL=p2p`, `hangup`, `group`, `conference`, `conference-legacy`, `conference-invite`, `incoming` or `incoming-group` | A voice call started or received, with a simulated voice server |
| `AURORA_DEMO_TAGS=voice`, `status`, `cycle`, `wall`, `height` or `render` | Name plates in all their states, at the three distances; occlusion, stable height and manual rendering limits |
| `AURORA_DEMO_CHAT_BUBBLES=1`, `typing` or `talk` | With `AURORA_DEMO_TAGS`: the residents talk, chat bubbles above the heads for this run; `typing`: the typing dots too; `talk`: as Preferences say |
| `AURORA_DEMO_STAFF=1` | A few demo residents count as Aurora team members or Linden Lab employees: their plate's light and their tag |
| `AURORA_DEMO_VOICE_MENU=1`, `louder` or `muted` | The world menu of a resident in our voice channel, with their voice volume row first |
| `AURORA_DEMO_VOICE_MODERATION=1` or `self` | Nearby voice moderation: the estate manager's entries, or a moderator muting us |
| `AURORA_VOICE_LEVEL_LOG=1` | Our own plate's voice level in the log, once a second (demo or grid) |
| `AURORA_DEMO_FRIEND_PLATES=1` | "Full plates for friends only" for this run, without saving it |
| `AURORA_DEMO_NAME_TAGS=<mode>` | Offline self/friend/other tag visibility and temporary-mode scenarios |
| `AURORA_DEMO_PLATE_HOVER=x,y[,click]` | Pointer held at x, y for the name plates, and a click there at frame 700 |
| `AURORA_DEMO_PLATE_CLICK=profile`, `im`, `tp` or `menu` | "Click on a plate" for this run, without saving it |
| `AURORA_DEMO_PLATE_JIGGLE=1` | "Jiggly name plates" for this run, the hover pointer going in and out of its plate |
| `AURORA_DEMO_PLATE_COMBO=<clicks>` | "Click combo counter" for this run, and that many clicks on the hovered plate |

### Details

- **`AURORA_DEMO_MINIMIZED_MUTE=1|off`**: overrides the Sound preference
  for this offline launch. Combine with `AURORA_DEMO_SOUND=1` for the
  looping world chime, or with local parcel/prim media or a chat video.
  With `1`, minimize the native window: all playback and microphone
  transmission must stop until restoration. Switching to another app alone
  must leave playback audible. `off` keeps playback audible while minimized.
  Restoration must preserve the saved channel volumes, mutes and microphone
  toggle. Debug logging reports `audio: minimized mute true/false`.

- **`AURORA_DEMO_VOICE`**: `speaking` animates the outline of our own name
  tag and of the other avatar's with realistic synthetic levels (0.35 to
  0.61 on the WebRTC scale), `loud` holds a strong level of 0.60: both
  outlines must be red without needing a saturated signal. `listening` keeps
  both name tags neutral, `off` shows the red struck-through microphone.
  `cycle` chains 4 s of speech, 3 s of listening, 2 s out of voice. Without
  this variable, our outline follows the real microphone when its button /
  push-to-talk is active and voice is enabled (`AURORA_DEMO_TALK=1` turns
  the microphone on; its meter is turned into the voice server's scale, so
  a voice reads as loud as on the grid). No saved setting is changed.
  With the name plates (default style), a speaking plate gets a ring that
  follows the level: a whisper is a faint hairline hugging the plate with
  almost no glow, a normal voice a green ring a few points away over a soft
  glow, a shout a thick bright ring pushed out with a wide glow, sliding
  through amber to red from -18 dBFS (RMS over 300 ms) to -12; normal
  speech at a sane microphone gain reads -30 to -24 and stays green. Out of
  voice, the plate has neither a coloured outline nor a badge. `sweep`
  keeps our own plate on the real microphone and, with
  `AURORA_DEMO_TAGS=voice`, makes Tess speak over the white wall, her voice
  going whisper → normal → shout → normal every eight seconds, next to
  Orion's normal voice over the dark scene: look at the ring on both
  backgrounds, through the whole range, and at the start and end of speech
  (never a jump).
- **`AURORA_DEMO_CALL`**: the demo has no voice server; `demo_calls.rs`
  answers ChatSessionRequest (a channel for every call) and stands in for
  the voice session: the call connects after 1.5 s and the others talk in
  turn, two seconds each (green ring around their picture). Every call made
  from the interface works with it, without the variable. Three seconds
  after arriving, `p2p` calls Tess: "Calling…" until she answers 3 s later,
  then "In call" with a timer; `hangup` is the same and Tess hangs up after
  12 s (back to nearby voice, a notice in the conversation); `group` calls
  the group Aurora Builders (Nova, Tess and Orion); `conference` starts a
  voice conference with Tess and Nova (`conference-legacy`: the capability
  answers 400 and the conference starts by message, IM_SESSION_CONFERENCE_START;
  the log says "starting it by message"); `incoming` makes Tess call us (the
  card with Accept, Decline and Reply by text, and the same answers in her
  conversation, which comes up flashing; unanswered, both close after 30 s)
  and
  `incoming-group` makes Nova start a call in Aurora Builders. Voice must be
  enabled (Preferences › Sound and voice), as on the grid. Residents added to
  a conference ("Add" of the call strip, or dragged from Contacts › Friends
  or People › Nearby onto the conversation) join it at once, and our
  message "<names> was / were invited to the conversation." follows in it
  (`conference-invite` adds Orion by itself once the conference is connected).
- **`AURORA_DEMO_STAFF`**: the members of the Aurora team (`world/staff.rs`)
  are marked everywhere Aurora shows a resident: on the name plate, green
  to violet curtains of light rise from behind the top edge, the border's
  colours flow around it over a soft halo, and the full and compact plates
  carry the "Aurora Viewer" tag over their top left edge (the far pill keeps
  the light alone); after the name in People (Nearby, Friends, Recent,
  Contact sets, the radar table), in the Conversations window (the name of
  a message, the head of a conversation) and on the profile, the same tag
  in the windows' corners, plus the light over the Aurora profile's
  banner. With "Reduce animations" the light holds still. The demo's
  members: ourselves, the first friend of the buddy list
  (`AURORA_DEMO_PROFILE=friend`, the conversation of `AURORA_DEMO_CONV=bubbles`),
  the resident who speaks in nearby chat (`AURORA_DEMO_CONV=bubbles:local`
  and the chat console), and with `AURORA_DEMO_TAGS=voice`
  Orion (full plate, speaking: the voice ring over the light), Sol (compact
  pill) and Vega (far pill). Linden Lab's employees (on the grid: the last
  name Linden) get their own mark in the Linden colour of Preferences ›
  Colors: four-pointed stars rising and twinkling above the plate, a shine
  running around the border, the "Linden Lab" tag with the verified
  rosette, and stars twinkling over the profile's banner. The demo's
  Lindens: the wolf (nearby chat, `AURORA_DEMO_PROFILE=wolf`), Lune of
  People › Near you, and with `AURORA_DEMO_TAGS=voice` Lune (full plate,
  typing) and Iris (compact pill).
- **`AURORA_DEMO_TAGS`**: nine residents in front of the starting position
  (`demo_tags.rs`), to see the name plates without a grid. Five are less
  than 15 m from the camera (full plate), two around 27 m (compact pill),
  two around 50 m (name only). A white wall stands behind the two on the
  left, to judge the glass on a light background next to the same plates on
  the dark scene. The People window is closed.
  - `render`: the voice scene with Tess (full plate) and Iris (compact pill)
    set to Never fully render through the ordinary rendering exceptions.
    Both have the crossed-out eye badge; Orion keeps the automatic-limit
    warning triangle. Brume keeps the separate blocked badge. Use
    `AURORA_NAME_TAGS=classic` to check the neighbouring classic tags.
  - `AURORA_DEMO_CHAT_BUBBLES=1|typing|talk` turns the chat bubbles above
    the heads on for the run (nothing saved; `typing` adds the typing dots;
    `talk` forces nothing, to try Preferences › Chat's options) and
    makes the residents talk every 30 s, 2 s after the start: Nova says a
    line, Orion whispers (italics), Tess shouts (bold), Lune emotes ("Lune
    … waves"), we ask something, Sol, farther, says a long line that wraps,
    Brume, blocked, gets no bubble, Nova adds a line, and a third at 14 s.
    Each tag rises above its bubble; at 60 fps frame 780 shows the first
    lines greyed (older than 8 s), frame 900 Nova's third line. Iris types:
    with `typing`, three dots in her bubble. The lines above the chat bar
    keep only the notices ("Not above the chat bar"). Use
    `AURORA_NAME_TAGS=classic` for the classic tags' look.
  - `height`: Nova and Lune have a torso translation deformer (+0.8 m),
    animated head pitch and pelvis bobbing. Their tags stay above the taller
    bodies, horizontally centred on their projected roots at a constant
    height through the cycle. With a pitched camera, check avatars on both
    sides of the view: raising a tag must not shift it sideways.
    Compare frames 660 and 780 at 60 fps; also use `AURORA_NAME_TAGS=classic`.
    A change of animation can change its height envelope; its individual
    frames never move the tag. Use a human-run grid session to verify real deformer assets.
  - `voice`: Nova out of the voice channel (no microphone badge);
    Tess in the channel, silent (green microphone badge, no outline) and
    without a profile picture (the icon stays); Orion, a friend, is speaking
    (green ring that follows the level, microphone, green heart
    and triangle badges: he is shown as too complex;
    his name is in ink like everyone's unless Preferences › Colors › "Color
    the names by relation" is on); Lune
    is typing (three animated dots) and has no display name (two lines); La
    Brume is blocked (dimmed plate, name in italics, forbidden badge, no
    status dot). Farther away, Sol is speaking (a tighter ring) and Iris is
    typing, without the typing animation (only her typing notices, as a
    viewer with the animation off); beyond, Vega is in the channel and Atlas is not (the far pill
    has no badge).
  - `status`: Nova away (amber dot), Tess do not disturb (red dot), Orion
    out of the channel (green heart and triangle), Lune typing
    while away.
  - `cycle`: switches from one to the other every six seconds.
  - `wall`: the states of `voice`, with the wall in front of Nova and Tess
    instead of behind them, and two violet boxes that carry the same
    floating text, one of them behind the wall. The scene hides the labels
    behind it: nothing of Nova's plate shows, the wall's edge cuts Tess's
    plate, Sol's pill and the floating text of the hidden box along a
    smooth line, and the box in the open keeps its text. Orion and those on
    the right are whole. `AURORA_LABEL_OCCLUSION=0` gives the image without
    the cut. To look at in `AURORA_AA=3`, `5` and `6` (with 5, the cut must
    not shimmer from one frame to the next).
  - With `AURORA_DEMO_FRIEND_PLATES=1` (the setting Preferences › Aurora ›
    "Full plates for friends only" for this run, not saved): only Orion, the
    friend, and our own plate are complete; Nova, Tess, Lune and La Brume are
    compact pills however near; the far pills do not change.
  - `AURORA_DEMO_NAME_TAGS=all|self-off|friends-off|others-off|off|temporary|classic-temporary`
    selects the visibility scenario at startup. Combine with `AURORA_DEMO_TAGS=voice`
    for a row containing a friend and other residents. Temporary modes use
    ten seconds and the final one-second fade; turn away and return to check
    a fresh interval. `AURORA_DEMO_OPTIONS=avatars` opens the new controls.
    These explicit scenarios use a 64 m tag distance and close the AO panel
    so earlier demo preferences cannot hide the fixture. Only the isolated
    demo preferences are affected.
  - `AURORA_DEMO_PLATE_HOVER=x,y` holds the pointer at x, y (points) for
    the plates only: the plate there shows full ("Expand a plate on hover",
    on by default), a pill or a far name included, and its surface and
    border are lighter (#473; the bounce on arrival is over before the
    capture, try it with the mouse); with `,click` the plate
    is clicked at frame 700. `AURORA_DEMO_PLATE_CLICK=profile|im|tp|menu`
    sets "Click on a plate" for the run (not saved): the pointer turns into
    the hand of a clickable object over another avatar's plate, and the click
    opens the profile, the conversation, the teleport offer box or the
    avatar's menu. For example, with `AURORA_DEMO_TAGS=voice`, `580,316` is
    Orion's plate and `198,334` Nova's pill once the friends-only option is on
    (`AURORA_DEMO_FRIEND_PLATES=1`).
  - Preferences › Just for fun (#1112, #1114), with
    `AURORA_DEMO_PLATE_HOVER=x,y` on a plate: `AURORA_DEMO_PLATE_JIGGLE=1`
    turns "Jiggly name plates" on for the run (not saved) and takes the
    pointer off the plate and back every 8 frames from frame 600 to 700: a
    capture at frame 600 to 700 catches the plate squashed or stretched and
    the pill overshooting as it grows. `AURORA_DEMO_PLATE_COMBO=<clicks>`
    turns "Click combo counter" on for the run and clicks the plate that many
    times, one every 4 frames, the last at frame 700 (`demo plate combo` in
    the log, with the resident's total): from five clicks the counter is out
    of the plate's bottom right corner with its "+1", at ten the burst and
    "×10" float up; it fades 2.5 s after the last click. The totals are kept
    in the demo profile (`accounts/<id>/plate-clicks.json`), so they grow
    from one run to the next. `AURORA_DEMO_OPTIONS=fun` opens the page and
    its sample plate.
  - The pictures are generated gradients (`demo_tags::picture_pixels`),
    requested as on the grid: profile, then texture, at most four per
    second, the nearest first; our avatar and La Brume have the same one.
  - To be combined with `AURORA_DEMO_VOICE` for our own plate and that of
    Loup Violet (blocked in the demo), and with `AURORA_NAME_TAGS=classic`
    to compare with the classic name tags.
- **`AURORA_DEMO_VOICE_MENU`**: at frame 600, opens the world menu on the
  nearest name plate of a resident in our voice channel whose voice is not
  blocked (with `AURORA_DEMO_TAGS=voice`: Orion, the speaking friend).
  Its first row is the resident's voice: speaker, volume slider, percentage
  (100 % = as they send it, 200 % at the end) and the voice mute button.
  `louder` sets their volume to 160 % first, `muted` blocks their voice
  first (the slider greyed at 0 %, the button lit). Capture at frame 620 or
  later with `AURORA_FPS_LIMIT=60`; `AURORA_DEMO_PIE=1` shows the same row
  in a panel under the pie menu. The list menus of the same residents get
  the row too, opened with `AURORA_DEMO_POINTER` (right click `,r` at
  frame 302, capture at frame 345): with `AURORA_DEMO_PEOPLE=rows`,
  `150,439,r` is Nova in People › Near you (in voice) and `150,304,r` Lune
  (out of voice: no row); with `AURORA_DEMO_PEOPLE=friends`, `150,240,r` is
  Nova in Friends and `688,237,l` the "…" menu of her sheet. With
  `AURORA_DEMO_CALL=group` and `AURORA_DEMO_CONV=aurora`, the menu of a
  participant's name in the conversation has it once the call is connected
  (`…;764,616,r` as the seventh point, capture at frame 720). Positions are
  window pixels at the default capture size; the residents out of voice
  never get the row.
- **`AURORA_DEMO_VOICE_MODERATION`**: `1` makes the avatar an estate
  manager on a parcel that uses the estate's voice channel: the voice row
  of a resident's menu (`AURORA_DEMO_VOICE_MENU=1`, with
  `AURORA_DEMO_TAGS=voice` for residents in voice) gets "Mute in nearby
  voice for everyone" (or "Unmute…" once muted) and "Mute / Unmute everyone
  in nearby voice"; the demo applies them at once, as the simulator would.
  `self`: a moderator mutes us at frame 700 and lets us speak again at
  frame 1300, each with its notification.
- **`AURORA_VOICE_LEVEL_LOG`**: once a second, a `voice level (own plate,
  loudest over 1 s)` line with the loudest raw level of our own plate (and
  its dBFS, the microphone meter's value), whether we spoke, the smoothed
  level, the red threshold and the color reached. Our own plate must be
  shown (Preferences, "Show my own name"); in demo with `AURORA_DEMO_TALK=1`
  and without `AURORA_DEMO_VOICE` (or with `sweep`), it follows the real
  microphone. On the grid it also confirms the scale of the levels against
  what the other residents' plates show.
- **`AURORA_DEMO_MEDIA_CLICK`**: focus click at frame 700 by default,
  second click 60 frames after, optional click on the field 120 frames
  after, then "aurora" is typed. `AURORA_DEMO_MEDIA_CLICK_FRAME` changes
  the starting frame to let the plugin load; also works with
  `AURORA_DEMO_HUDS=media`.
- **Media plugins.** The media scenarios need Aurora's media plugins pack
  (SLPlugin, CEF, LibVLC; on Linux GStreamer instead of LibVLC, from the
  system). A viewer that finds no plugins downloads it once
  from the Aurora release `media-plugins-<n>` into
  `%LOCALAPPDATA%\Aurora\AuroraViewer\data\media-plugins\<n>`, in demo mode
  too; every later run and every worktree reuses it. The media wait for it
  and start by themselves; the log says `media: downloading the plugins
  pack` then `media: plugins in …`. An installed Firestorm is no longer
  used, unless its folder is set in Preferences › Sound and voice › Media.
- **`AURORA_DEMO_MEDIA_PACK`**, with `AURORA_DEMO=1`: shows a state of that
  download without the network, whatever is installed (the media then have
  no plugins). `downloading` is at 45 %, `installing` is the extraction,
  `failed` shows the error and its "Retry" button. To look at with
  `AURORA_DEMO_OPTIONS`, in the Media group of the "Sound and voice" tab,
  and in the "Feed" tab of a profile, which shows the same progress
  instead of the page.
- **`AURORA_MEDIA_PACK_FEED`**: to try a pack before it is published (the
  artifact of a "Media plugins" workflow run). The viewer asks
  `<feed origin>/download/media-plugins-<n>/Aurora-Media-Plugins-<n>-windows-x64.zip`
  (`-linux-x64.zip` on Linux)
  instead of the GitHub release; only a loopback address is accepted, and
  the archive must still have the size and the SHA-256 written in
  `crates/aurora-updater/src/media_pack.rs`. A pack installed that way is
  replaced when these two values change.
- **`AURORA_DEMO_MUSIC`**: `1` designates an unreachable dummy address; the
  music waits for the radio button.
- **`AURORA_DEMO_MEDIA_FILTER`**: turns the media filter on for the run.
  `ask` (or `1`) starts a parcel media on `media.example.com`, which the
  filter asks about (the question is up from the first frames; nothing
  loads until it is answered), `first` the same with the first use's
  explanation and its "Do not filter media" button, `music` starts the
  parcel music on `radio.example.org` (the log has no `parcel music:` line
  before the answer), `list` puts three sites in the list of Preferences ›
  Preferences › Media (`AURORA_DEMO_OPTIONS=media`). In the demo the
  list and the answers stay in memory: no run changes the demo account's
  `medialist.xml`, and an answer does not save the profile. `AURORA_DEMO_MEDIA=<https URL>` or
  `AURORA_DEMO_PARCEL_MEDIA=<https URL>` ask the same way when the
  profile's filter is on. `worn` puts media on `media.example.com` on the
  object worn on our back (that of `AURORA_DEMO_WORN`), `hud` on a worn HUD
  screen: no question, as "Trust my worn items" is on (the log has no
  `media filter: asking about` line). `worn-ask` / `hud-ask` turn that
  option off for the run, and the question comes; `worn-blocked` /
  `hud-blocked` put the site in the list as blocked: refused without a
  question, told in the local chat.
- **`AURORA_DEMO_SOUND_EXPLORER=1|all|blacklist|blacklist-ignored|avatar-menu|avatar-menu-mixed|avatar-menu-pie`** exercises Sound Explorer
  with a looping object, triggered avatar and default collision sound. `1`
  keeps the default filters; `all` shows avatars, collisions and repeated
  assets. The fixtures use the normal sound manager. Check pause/resume,
  Ctrl/Shift selection, local preview/stop, source camera and blocking; open
  Asset blacklist to remove a sound or owner category. Add
  `AURORA_DEMO_SOUND=1` to hear the fixtures. Grid validation still requires
  real scripted objects, gesture sounds and a relog for account persistence.
  `blacklist` instead opens the blacklist with one resident's three sound
  categories and an unrelated asset, held in memory without changing the
  demo account's saved list. Each category has its own line and selection;
  removing one must leave the other two, and removing the last must leave
  the unrelated asset. Check type labels, Ctrl/Shift selection and both the
  Remove selected button and row context menu.
  Check the full UUID column at normal and narrow window sizes, searching by
  UUID (also uppercase), sorting and Copy UUID in the row menu. A sound asset
  shows its own UUID, while owner category rows show the owner's UUID; copying
  several categories of one owner copies that UUID once. The Ignored objects
  and residents button opens People › Blocked with its filter cleared, where
  a separate in-memory ignored object fixture is listed. `blacklist-ignored`
  opens that destination directly through the same app action. These fixtures
  do not write either the asset blacklist or the mute cache. On the grid,
  confirm that Ignore lists an object under People › Blocked and Block sound
  lists the sound asset UUID in Asset blacklist; remove each through its own
  list and verify account persistence after relog.
  `avatar-menu` opens the other avatar's world menu with all three sound
  categories OFF. `avatar-menu-mixed` starts with worn/gesture sounds ON and
  rezzed-object sounds OFF; `avatar-menu-pie` exercises the same actions in
  the radial menu. Open Disturbance and toggle each category in both directions:
  ON blocks it, OFF removes only that category, and the list menu stays open.
  The fixture blacklist stays in memory. Use AURORA_DEMO_POINTER to hover the
  submenu or activate its controls. On the grid, check the real sounds, the
  separate blacklist rows and persistence after relog. Sound filtering is local
  and does not use the Bridge; blocking an object does not inherit to rez descendants.

- **`AURORA_DEMO_ANIMATION_EXPLORER=1|blacklist`** opens the Animation
  explorer over three rows: a plaza object named "Dance ball" still dancing
  our avatar (selected, so the stand-in avatar dances), a "Greeter HUD" whose
  motion stopped at once (its "Played" time counts up) and our own stand.
  Check "No owned animations", the column sorts, the preview's orbit, zoom
  and reset, and that Stop / Stop and revoke / Blacklist animation act on the
  selected row. `blacklist` instead opens the Asset blacklist with the HUD's
  motion as an Animation row, held in memory only. Grid validation still
  needs real pose balls, HUDs and a relog for the saved blacklist.

- **`AURORA_DEMO_STREAM_TITLE=[window|history|text][,stop]`**: the demo
  parcel's radio changes title every 6 seconds, one title too long for the
  window (it slides to show its end, waits, comes back), after five titles
  played during the loading, so the history is filled. At frame 600
  World › Stream title opens: the window (`window`, the default), the
  window and its history (`history`: newest first, Ctrl / Shift to choose,
  Clear, Max. entries, Copy to clipboard), or the title alone over the
  world (`text`: drag it anywhere, right click for the history, the copy
  and the window). `stop` ends the stream after the last title: "No
  stream playing", the history kept. No sound is played.
- **`AURORA_DEMO_SOUND=1|hud|parcel-open|parcel-local|parcel-source`**: world sounds (looping chime) and interface
  sounds (taken from the real sound cache if it exists, otherwise a brief
  tick per sound). Without it the media play muted too: the viewer sends
  them a volume of 0, so a video in the demo is silent unless this is set.
  The wind sound is silent too, unless this or `AURORA_DEMO_WIND` is set.
  With `hud`, the chime loops on a linked HUD prim instead of the world
  objects. Move the camera away or turn it: the chime stays centered at the
  same volume. Effects volume and mute still control it; hiding the HUDs
  does not stop their sounds.
  The `parcel-*` scenarios place a looping cube just west of the x=128 m
  boundary and our avatar on the northeastern parcel (x=132, y=132).
  `parcel-open`: the neighbour's chime is audible. `parcel-local`: our
  parcel restricts sounds, so the chime is silent. `parcel-source`: only
  the source parcel restricts sounds, with the same silence outside it.
  The cube, other world audio settings and interface sounds remain usable.
  Live crossings, changing either parcel's restriction and a real rain
  emitter still need human grid validation.
- **`AURORA_DEMO_SOUND_OCCLUSION=1`**: our avatar starts on an open slope
  west of the plaza (108, 150), three looping sources ahead of it, each
  with its own pitch, lit, and a label: on the left one in
  the open, straight ahead one behind a light grey wall, on the right one shut in
  a closed grey box. From the start the middle and right ones are dull and
  quieter (low-pass and gain cut), the left one clear; walk past the end of
  the wall and the middle one opens up over a tenth of a second, without a
  click; the box stays muffled from every side. Preferences › Sound, "Muffle
  sounds behind walls" off: all three clear at once. The log gives each
  sound's amount when it changes (`sound occlusion: voice <n> <amount>
  (<layers> layers) from <object>`; 0 clear, 0.5 one layer, 1 two or more),
  at info level in this scenario (debug otherwise); with `AURORA_PROFILE=1`,
  a `sound occlusion summary` line every ten seconds gives the measures'
  cost on the main thread and in their job. Sounds play without
  `AURORA_DEMO_SOUND`.
- **`AURORA_SOUND_OCCLUSION=0|1`**: overrides Preferences › Sound, "Muffle
  sounds behind walls", for this launch (demo or grid), without saving it:
  the same build measured with and without the occlusion.
- **`AURORA_DEMO_SOUND_HRTF=1|circle|updown`**: listen with headphones. Our
  avatar starts on the open slope west of the plaza (the occlusion
  scenario's spot: run the two one at a time) and two looping sources move
  around the camera, 4 m away, each a lit cube with a label: "Circling
  sound" (violet, brushed noise bursts on a soft tone) turns around the
  listener level with the ears, clockwise from ahead (ahead, right, behind,
  left) in 16 s; "Rising and falling sound" (orange, bright clicks on a
  bell) swings ahead from 40 degrees below (above the ground at most) to 80
  degrees above and back in 10 s. `circle` or `updown` plays one alone,
  clearer for a first listen; `1` both. The circle starts from, and the
  swing faces, the direction the camera looks at the start (east-north-east);
  both follow the camera when it moves.
  With Preferences › Sound, "3D sound for headphones (HRTF)" on, the
  circling sound is heard passing behind the head (duller, out of the head)
  and not only swinging between the ears, and the swinging one is heard
  rising and falling; turning the camera moves both around the head without
  clicks or a zipper sound. Off, both are only panned between left and
  right, as before (the Legacy Viewer's rendering). Sounds play without
  `AURORA_DEMO_SOUND`.
- **`AURORA_SOUND_HRTF=0|1`**: overrides Preferences › Sound, "3D sound for
  headphones (HRTF)", for this launch (demo or grid), without saving it.
- **`AURORA_DEMO_SOUND_REVERB=1|room|hall`**: turn Preferences › Sound,
  "Reverb from the surroundings" on (or launch with
  `AURORA_SOUND_REVERB=1`). Our avatar starts in the open air south-east of
  the plaza, by a small lit post labelled "Open air" (`room` and `hall`
  start inside those instead); a violet "Clapper" cube floats above its
  head and claps twice every two seconds, wherever it goes. Ahead on the
  left stands a small closed room (7 x 7 x 3 m inside, its doorway on the
  side facing the path), straight ahead a large hall (22 x 30 x 12 m, a
  wide doorway facing the start); both are light grey boxes with their name
  above the doorway. Walk in and out (the listener is the camera: if the
  third-person camera ends up in or behind a wall, zoom in or use
  mouselook). In the open the claps are almost dry; in the small room they
  get a short, close ring (about half a second); in the hall a long, soft
  tail (about two seconds) that keeps sounding between the claps. Walking
  through a doorway changes the reverb over a second or two, without a
  click; the setting off makes every clap dry again, fading the tail out.
  The log gives the estimate and the parameters when they change
  (`sound reverb: enclosure <0..1>, open <0..1>, mean <m> (measured ...):
  level <0..1>, decay <s>, pre-delay <ms>, damping <0..1>, path <m>`), at
  info level in this scenario (debug otherwise); with `AURORA_PROFILE=1`, a
  `sound reverb summary` line every ten seconds gives the measures' cost on
  the main thread and in their job, and the per-frame cost of the sounds'
  sends. Sounds play without `AURORA_DEMO_SOUND`.
- **`AURORA_SOUND_REVERB=0|1`**: overrides Preferences › Sound, "Reverb from
  the surroundings", for this launch (demo or grid), without saving it: the
  same build measured with and without the reverb.
- **`AURORA_DEMO_WIND=<x>,<y>[,fly]`**: the demo region gets a uniform wind
  of x, y m/s (default `3,2`; LLWind::getVelocity doubles it, as on the
  grid), and with `fly` our avatar arrives flying. The wind noise plays on
  the Ambient channel only while flying above the water: hovering still you
  hear half the region's wind (AudioLevelWind 0.5), flying forward makes it
  louder and higher, turning sideways to the wind moves it to one ear, and
  landing (or going under the water) fades it out over two seconds. The log
  says `wind sound: fading in` / `fading out` at each change.
- **`AURORA_DEMO_VOLUME`**: `open` keeps the volume panel of the speaker
  open as if the pointer were on it (the panel normally opens on hover and
  closes 0.35 s after the pointer leaves it); `muted` also draws the speaker
  struck through, as after a click on it. Display only: the volumes and the
  mutes of the settings do not change. Look at the panel: only Music,
  Multimedia and Voice have an enable box.
- **`AURORA_PERCEPTUAL_VOLUME=0|1`**: Preferences › Sound › "Perceptual
  volume sliders" forced off or on for the launch, without saving it
  (`widgets::volume_slider`, #463). With `1` the knobs of the same volumes
  sit further right (a 50 % gain at about four fifths of the travel, a
  resident's 100 % still in the middle), the percentages unchanged. With
  `AURORA_DEMO_VOLUME=open`, capture once with `0` and once with `1`.

## Forced settings and diagnostics

| Variable | Effect |
|---|---|
| `AURORA_OCCLUSION=0` or `1` | Overrides the "Occlusion" setting (Graphics › Quality) |
| `AURORA_NO_OCCLUSION=1` | Turns off GPU occlusion |
| `AURORA_MSAA_PREPASS=0` or `alt` | MSAA without the prepass of its depth, or both alternately |
| `AURORA_NOVSYNC=1` | Turns off vertical sync |
| `AURORA_PRESENT_MODE=<mode>` | Forces the present mode of the swapchain |
| `AURORA_WINDOW_HOLD=0` | No frames while a mouse button is held on the window frame or the system menu is open, as before (Windows) |
| `AURORA_CPU_CULL=1` | Draw lists built on the CPU |
| `AURORA_LIST_INDEX=0` or `check` | Draw lists that walk all the objects as before, or check of their index |
| `AURORA_TEXTURE_TODO=0` or `check` | Texture streaming that walks every texture as before, or check of its to-do list |
| `AURORA_CPU_ATTACH=1` | Worn objects and rigged meshes placed by the sync, without the GPU anchors |
| `AURORA_WORN_RANK=own` | Meshes worn by other avatars downloaded in the rank of their own size, not their avatar's, as before |
| `AURORA_FETCH_SLOTS=<n>` | Asset requests under way at once fixed for this launch (1 to 256), whatever Preferences › Network and cache › Download connections says |
| `AURORA_DEMO_NET_ADAPTER=<description>` | This network adapter instead of the computer's, for the download connections (e.g. `Marvell AQtion 10Gbit Network Adapter`: always limited, "Follow the connection" greyed) |
| `AURORA_SYNC_SLICE=1` or `alt` | Scene sync by slice of all the objects, as before |
| `AURORA_PENDING_PARK=0` | Every object waiting for its geometry asked about by the sync's round, as before |
| `AURORA_SYNC_CHECK=1` | What the scene sync visits each frame |
| `AURORA_SYNC_SLOW=<ms>` | The steps of each frame whose scene sync is slower than `ms` |
| `AURORA_MATERIAL_REBUILD=all` or `alt` | Every object using materials rebuilt at each material change, as before |
| `AURORA_IMPOSTOR_POSES=every` or `alt` | Impostors posed at every frame, as before |
| `AURORA_ANCHOR_CENTRES=every` | The centre of every worn object recomputed at every frame, as before |
| `AURORA_NAME_TAGS=classic`, `plate` or `alt` | Style of the avatar name tags for this launch |
| `AURORA_LABEL_OCCLUSION=0` | Name tags and floating texts drawn over the scene, as before, instead of behind what is in front of them |
| `AURORA_TEXTURE_BC7=0` | Textures never compressed to BC7 |
| `AURORA_TEXTURE_BC7_CACHE=0` | BC7 without its disk cache |
| `AURORA_RENDER_THREAD=0` | Frames drawn on the main thread |
| `AURORA_FRAME_PACING=0\|always` | No frame pacing (the input waits in the frame queue when the GPU sets the pace), or the wait on every frame (0.8.4) |
| `AURORA_FRAME_PRIORITY=normal` or `alt` | The frame's threads at normal priority, or both priorities alternately |
| `AURORA_ALLOCATOR=system` | The system's memory allocator instead of mimalloc, as before |
| `AURORA_EYE_RELATIVE=0` | Vertices of the objects and the terrain computed in region coordinates |
| `AURORA_SHADOW_LEGACY=1` | Shadows drawn by the old path |
| `AURORA_TANGENTS=vertex\|pixel\|derivative\|alt` | Tangent frame of the normal maps: the vertices' (default), per pixel, the former one, or vertex and pixel alternated each second |
| `AURORA_ALPHA_COVERAGE=0` | Mips of alpha-masked textures plainly averaged, as before (foliage thins out with the distance) |
| `AURORA_MIP_FILTER=weighted` | Colour of the mips averaged by the alpha, as before (distant foliage brighter than in the Legacy Viewer) |
| `AURORA_SHADOW_FILTER=gaps\|mapped\|before` | Shadow lookup of before (lines along the shadow edges), to compare |
| `AURORA_SHADOW_BIAS=legacy\|new\|alt` | Shadow receiver bias of before (stairs on avatars' backs), or one second in two |
| `AURORA_BLEND_SHADOWS=0` | Alpha-blended faces that are not rigged cast no shadow, as before |
| `AURORA_DEBUG_SHADOWS=1` | Shadow view: the cascade each pixel reads, and its shadow |
| `AURORA_PROFILE=1` | Profiling in the log (see [Profiling](#profiling)) |
| `AURORA_FOOT_IK_SCOPE=<mode>` | Offline foot-placement benchmark override (all or visible10) |
| `AURORA_PROFILE_FRAMES=1` | Detailed profile of each frame |
| `AURORA_OWN_MOTION_TRACE=1` | Aurora's movement of our avatar frame by frame on a grid |
| `AURORA_GPU_VALIDATION=1` | wgpu validation layers |
| `AURORA_VULKAN_LAYERS=keep` | Loads the third-party Vulkan layers the viewer keeps out (Overwolf's, and the other overlays' unless allowed in Preferences) |
| `AURORA_VULKAN_DRIVERS=keep` | Loads Dozen (Vulkan on Direct3D 12) when installed, which the viewer keeps out |
| `AURORA_GPU=<number>`, `<name>` or `auto` | The GPU to draw with for this launch, over Preferences › Graphics |
| `AURORA_GPU_START_FAIL=1` | Start as if no graphics card could draw the window: the message the user gets then |
| `AURORA_PIPELINE_CACHE=0` | No pipeline cache of Aurora's own: every pipeline compiled by the driver, or taken from its own cache, as before |
| `AURORA_HOVER_CHECK=1` | Diagnostics of the hover cursor |
| `AURORA_CLICK_PICK=wait` or `check` | Depth under a click read by waiting for the GPU, or checked |
| `AURORA_HOVER_LEGACY=1` or `ab` | Search for the object under the cursor as before the classes and the triangle hierarchies |
| `AURORA_STILL_CHECK=1` | What changes when the image should be still |
| `AURORA_SCULPT_WATCH=<avatar UUID>` | Logs changed attachment geometry and sculpt-map states for one avatar |
| `AURORA_DEMO_DESCRIBE=<frame>[,clipboard]` | Opens the Dev Toolbox and presses J1 (J2) at that frame |
| `AURORA_DEBUG_GLOW=1` | Glow diagnostics |
| `AURORA_GLOW_SKIP=<mask>` | Removes a family of faces from the glow |
| `AURORA_MEDIA_DEBUG=1` | Media diagnostics |
| `AURORA_FONT_FALLBACK=0` | Without the system fallback fonts |
| `AURORA_UI_FONT=<font>`, `AURORA_UI_SCALE=<scale>` | Interface font and scale for this launch, without changing the Preferences |
| `AURORA_TEXT_SIZE=<size>` | Text size for this launch, without changing the Preferences |
| `AURORA_TEXTURE_BUDGET_MB=<n>` | Forces the texture memory budget |
| `AURORA_VRAM_BUDGET_MB=<n>` | The driver's video memory budget taken as n MB, a smaller card on a large one |
| `AURORA_TEXTURE_SIZING=screen` | Textures sized by their object's diameter on screen times the repeats, as before the Legacy Viewer's sizing |
| `AURORA_TEXTURE_SLOTS=<n>` | Size of the texture table, to test a full one |
| `AURORA_BANDWIDTH_KBPS=<n>` | Bandwidth setting for this launch, without changing the Preferences |
| `AURORA_THROTTLE_MAX_KBPS=<n>` | Most bandwidth asked from the simulator for this launch (6,000 by default, up to 50,000) |
| `AURORA_LIGHT_TILES=0`, `alt` or `check` | Local lights without sorting by screen tile, or check of the sort |
| `AURORA_PROBE_WALK=list`, `0`, `alt` or `check` | Reflection probes without sorting by screen tile, or check of the sort |
| `AURORA_BLEND_EARLY_DISCARD=0` or `alt` | Invisible fragments of transparent faces lit before being discarded |
| `AURORA_PBR_OCCLUSION=0` or `alt` | PBR materials without their own occlusion, as before |
| `AURORA_AO=legacy`, `gtao` or `alt` (`:<0-3>`) | Ambient occlusion method (and GTAO quality) for this launch |
| `AURORA_GTAO_SPECULAR=0` or `alt` | GTAO without its specular occlusion (reflections in creases), as before |
| `AURORA_GTAO_TEMPORAL=0` or `alt` | GTAO without its accumulation over the frames (fixed noise, finer grain), as before |
| `AURORA_GTAO_NORMALS=depth` or `alt` | GTAO with the normal rebuilt from the depth (facets on low-poly meshes), as before |
| `AURORA_DEBUG_MOTION=1` | The screen's motion vectors shown instead of the image |
| `AURORA_SSIL=1`, `0` or `alt` | GTAO's screen-space indirect light forced on or off for this launch |
| `AURORA_SSIL_INTENSITY=<0-8>` | Strength of GTAO's indirect light for this launch |
| `AURORA_CONTACT_SHADOWS=0`, `1`, `alt`, `prepass` or `shell` | Sun contact shadows off, on, by turns, cast by the depth prepass alone, or by thin shells |
| `AURORA_CONTACT_SOFT=0` | Contact shadows with hard pixel edges (no softening) |
| `AURORA_CONTACT_VARIANT=no-early-out`, `no-shared`, `short` or `skip` | Diagnostic: the contact shadows' shader without one of its constructs (GPU hangs on some AMD cards) |
| `AURORA_BLOOM=0`, `1` or `alt` | Bloom off, on or one second in two for this launch |
| `AURORA_AUTO_EXPOSURE=0`, `1` or `alt` | Automatic exposure of non-classic skies off, on or one second in two |
| `AURORA_SPECULAR_AA=0`, `1` or `alt` | Specular anti-aliasing off, on or one second in two for this launch |
| `AURORA_COLORED_AMBIENT=0`, `1` or `alt` | Sky ambient softened, grey under linear and HDR skies (Legacy Viewer), coloured at full strength like the first PBR release, or one second in two |
| `AURORA_RIGGED_BLEND=split`, `runs`, `copy`, `depth05`, `write`, `sorted` or `alt` | Depth of the blended faces of rigged meshes: object by object, what is half covered writing its depth as drawn, then the fainter texels, then their depth from 1/255 on a copy (default); the same without the fainter texels' depth (#1024); where nearly opaque before their colour, then point by point where half covered (#966); from 1/255 on a copy (#948); where half covered before their colour; from 1/255; none (sorted with the others); or `split` by turns |
| `AURORA_MESH_ORDER=asset` | Mesh faces keep the triangle order of the asset instead of the Legacy Viewer's vertex cache order |
| `AURORA_BLEND_AO=new`, `none`, `old` or `alt` | Which blended faces take the screen-space occlusion and indirect light: those at least half covered (default), none, all, or `none` by turns |
| `AURORA_DEMO_VIEWER_STATS=1` | Writes once to the log the viewer statistics report the grid would receive (demo mode, nothing sent) |

### Details

- **`AURORA_DEMO_VIEWER_STATS=1`** (`aurora_net::viewer_stats`,
  `app/viewer_stats.rs`): on the grid the session posts Linden Lab's viewer
  statistics to the region's `ViewerStats` capability once the agent has
  arrived, every 5 minutes, and at logout (send_viewer_stats of the Legacy
  Viewer). In demo mode nothing is posted: about 5 s after the world shows,
  the log gets one line `demo viewer statistics (not sent): <llsd>…` with the
  report built from the demo's real frame, machine and GPU values (the
  network counters are the demo's; the session id is never in it, and the
  adapter address and the serial show as `(not logged)`). On the
  grid the report is logged at debug level only, `sending viewer
  statistics: …` (`RUST_LOG=info,aurora_net=debug`).
- **`AURORA_VULKAN_LAYERS=keep`**: at start the viewer sets the variables
  that disable Overwolf's implicit Vulkan layers (`VK_LAYER_OW_OVERLAY`,
  `DISABLE_VULKAN_OW_OVERLAY_LAYER=1`; `VK_LAYER_OW_OBS_HOOK`,
  `DISABLE_VULKAN_OW_OBS_CAPTURE=1`) in its own process, unless the user
  already set them: their overlay chained into a capture hook crashed in
  `vkCreateInstance` one second after start (#901). Unless Preferences ›
  Graphics › Overlays › "Allow other programs' overlays" is on (read from
  `settings.json` at start, applied at the next start), it also disables
  the other overlays' layers: the Steam overlay
  (`DISABLE_VK_LAYER_VALVE_steam_overlay_1`), RivaTuner (`DISABLE_RTSS_LAYER`),
  the Epic Games overlay (`EOS_OVERLAY_DISABLE_VULKAN_WIN64`), Twitch Studio,
  FPS Monitor and PlayClaw. The log says so
  (`Vulkan layers kept out of the viewer`, each layer with its program,
  and whether overlays are allowed). `keep` leaves the environment
  as it is. OBS Studio's game capture (`VK_LAYER_OBS_HOOK`, and Wallpaper
  Engine's copy, which declares the same variable), ReShade and the GPU
  drivers' own layers are never disabled; `DISABLE_VULKAN_OBS_CAPTURE=1`,
  set by the user, disables OBS's. Overlays that inject a DLL into the
  running viewer instead (Discord) cannot be kept out: the log names them
  when they arrive (`overlay hook loaded in the viewer`). To
  see which layers the loader loads: `VK_LOADER_DEBUG=layer` in a console.
- **`AURORA_VULKAN_DRIVERS=keep`**: at start the viewer sets
  `VK_LOADER_DRIVERS_DISABLE=*dzn*` in its own process, so that the Vulkan
  loader skips Dozen (`vulkan_dzn.dll`, Vulkan on Direct3D 12, from
  Microsoft's "OpenCL, OpenGL, and Vulkan Compatibility Pack"): while it
  lists its devices, Dozen opens every Direct3D 12 adapter, and an old AMD
  Direct3D 12 driver crashed there at start (`vulkan_dzn` → `d3d12core` →
  `amdxc64` in the crash report). The log says so (`Vulkan driver kept out
  of the viewer`). Nothing is set when the user already set
  `VK_LOADER_DRIVERS_DISABLE`, `VK_LOADER_DRIVERS_SELECT`, `VK_DRIVER_FILES`,
  `VK_ICD_FILENAMES` or `VK_ADD_DRIVER_FILES`; a loader older than 1.3.234
  ignores the variable. When no Vulkan adapter is left without Dozen (a GPU
  with no Vulkan driver of its own), the renderer lets it back in and tries
  again (`GPU: no Vulkan adapter without Dozen` in the log). `keep` leaves
  the environment as it is. To see which drivers the loader loads:
  `VK_LOADER_DEBUG=driver` in a console.
- **`AURORA_GPU=<number>|<name>|auto`**: the adapter the renderer opens,
  over Preferences › Graphics › Fluidity › Graphics card (#894): its number
  in the `GPU adapter <n>:` lines of the log, or a part of its name in any
  case (`nvidia`, `intel`); `auto` or empty for the automatic choice
  (discrete before integrated, then the most video memory, among the
  adapters that can present to the window). An adapter that is missing or
  cannot present gives a warning and the automatic choice; one that fails
  to open hands over to the next. The log lists every adapter Vulkan
  shows, with its type, memory and whether it can present, then
  `GPU adapter <n> chosen:` and why. When only an integrated GPU is listed
  on a PC that has a dedicated one, the NVIDIA Control Panel's "Preferred
  graphics processor" for `aurora-viewer.exe` hides it from Vulkan
  (`VK_LAYER_NV_optimus`): the log says so, and Preferences › Graphics ›
  Fluidity › Graphics card shows a note under its title (the same for a
  dedicated card that cannot present). The list of that choice comes from
  the renderer's own list as soon as it is created, so the login screen has
  it too.
- **`AURORA_GPU_START_FAIL=1`**: the renderer is not created, as when no
  adapter can present or Vulkan is missing. Before #894's follow-up the
  viewer exited silently in that case, its window still hidden (no crash
  report: it is no crash); it now shows a native error box with what to try
  and the log's path, then exits. Automated captures (`AURORA_CAPTURE`) skip
  the box. The same box shows when the window itself cannot be created.
- **`AURORA_PIPELINE_CACHE=0`**: the renderer makes its pipelines without
  Aurora's cache (`aurora-render/src/pipeline_cache.rs`), as before: the
  driver compiles each one, or takes it from its own disk cache when that
  still holds it. To see a cold start without emptying the driver's cache,
  point NVIDIA's at an empty folder for one launch:
  `__GL_SHADER_DISK_CACHE_PATH=<empty folder>` (honoured by the Vulkan
  driver on Windows, checked 2026-11-01: the folder fills, and the next
  launch with the same folder is warm). The log says `pipeline cache: … read
  for <n> pipelines`, `… not used (<reason>)` or `none yet`, then
  `pipelines at start: <n> made in <ms>` with the three slowest. Its file
  is `<cache>/pipelines/<wgpu key>.bin` (`cache\demo\pipelines` in demo).
- **`AURORA_BANDWIDTH_KBPS=<n>`**: the "Maximum bandwidth" of Preferences ›
  Network and cache for this launch, without changing the saved setting,
  from 100 to 10,000. A session first asks its simulator for one and a
  half times the setting, 6,000 kb/s at most in all
  (`aurora-net/src/throttle.rs`): the default, 4,000, already asks for
  that most, and 3,000 is Firestorm's default.
  `AURORA_THROTTLE_MAX_KBPS=<n>` (6,000 to 50,000) raises that most for
  the launch, to measure what a simulator sends when asked for more than
  the Legacy Viewer ever asks: 8,000 with a ceiling of 12,000 asks for
  12,000. To compare what a simulator
  sends under two settings: on an arrival in a region that is not in the
  cache, read `udp in … pkt/s … kb/s` in the `net summary` lines, the
  `objects=` of `perf summary`, the `avatar … shown after` lines, and the
  `simulator bandwidth:` lines, which tell what is asked at first and at
  each change (losses, or the viewer falling behind).
- **`AURORA_NAME_TAGS`**: `classic` or `plate` imposes the style of the
  avatar name tags (classic or name plates) without touching the "Name tag
  style" setting. `alt` alternates within the same session, at each
  `perf summary` line: one second with the classic name tags (`xmode=old`),
  two with the plates (`xmode=new`). This is the measurement to make on a
  shared machine: same avatars, same camera, same outside load on both
  sides. To read: `u_tags` (main thread), `r_tessellate`, `r_egui`,
  `ui_meshes` and `ui_vertices`. Measurement scene: `AURORA_DEMO_CROWD=40`
  with `AURORA_DEMO_POS="135,134"` (in the crowd: the eight full plates,
  the rest as compact pills) or `"129,134"` (in front of the crowd),
  `AURORA_FPS_LIMIT=120`. With a still camera, a plate that has not moved
  is redrawn without a copy: the cost in motion shows when turning the
  camera during the reading.
- **`AURORA_LABEL_OCCLUSION=0`**: the avatar name tags (plates and classic)
  and the floating texts are no longer compared with the scene's depth
  (`aurora-render/src/label_depth.rs`): egui-wgpu draws them over the scene,
  as before, and they show through walls. The image only differs where the
  scene is in front of a label. Scene: `AURORA_DEMO_TAGS=wall`; cost:
  `r_egui`, `gpu=` and `draws=` of `perf summary` in a crowd
  (`AURORA_DEMO_CROWD=40`, `AURORA_DEMO_POS="129,134"`).
- **`AURORA_TEXTURE_SIZING=screen`**: the textures of a face are loaded for
  the diameter of its object on screen times the face's repeats, as before
  the Legacy Viewer's sizing (`scene/texture_importance.rs`: the circle
  around the object seen from its nearest point, up to eight times the
  area for an object within 16 m and in the middle of the view, divided by
  the repeats squared; no boost under memory pressure). To compare the
  two: the `texture load:` line (decodes per discard level, Mpx, MB
  downloaded) and `texture_mb` / `pages_mb` of `perf summary`, for example
  with `AURORA_DEMO_STREAM=200,200,j2k=<cache\tex>,net=200,size=4` and
  `AURORA_DEMO_POS="141,126"` (the first objects 5 m away).
- **`AURORA_TEXTURE_BUDGET_MB=<n>`**: the texture pages must not exceed
  n MB, whatever the "Texture memory" setting and the video memory the
  driver announces (`scene/texture_budget.rs`). Used to put the viewer
  under memory pressure on any machine, in demo as on the grid.
  - Above the target, the log announces each change of step
    (`texture memory: pressure 0 -> 1 (… MB in pages for a target of … MB, …
    lowered so far)`): 1 brings the textures back to their on-screen size,
    2 to 7 then remove half a level at a time, up to two levels, off-screen
    first. Each step lowers only what brings the pages back to 85 % of the
    target, the largest textures first. The same line gives the viewer's video memory: its resources,
    what the driver counts for it and the budget granted. The full
    Performance window shows the target and the step ("Texture budget", in
    Memory and streaming).
  - In demo, with `AURORA_DEMO_STREAM=3000` (4.7 GB of textures):
    `AURORA_DEMO_POS="168,126"` places the avatar in the middle of the
    objects; 512 climbs to step 3 and settles near 440 MB of pages, 96
    climbs to step 6 or 7. Without `AURORA_DEMO_POS`, 3000 goes over the
    target once, by a few megabytes: about 460 textures are lowered, 2.5 GB
    stay, and the step comes back to 0 (before the lowering was limited to
    the goal: 3,571 lowered, 105 MB left). The textures of this scenario are then reloaded on demand, like
    those of the grid (camera moved closer after a reduction). A huge value
    (1000000) removes all pressure, to compare.
- **`AURORA_VRAM_BUDGET_MB=<n>`**: the budget the driver grants the viewer
  is taken as n MB (at least 256), what the texture budget and the
  Performance window then read (`aurora-render/src/vram.rs`); what the
  driver counts for the viewer stays the real figure. It replaces the
  budget given out, the driver's lowest of the last minute (log line
  `GPU memory budget: … given out`), which a smaller card would not show
  either.
  - In demo, `AURORA_DEMO_STREAM=3000` with `AURORA_VRAM_BUDGET_MB=2048`
    behaves like a small card: the texture target leaves 20 % of the
    budget free with the rest of what the viewer holds (render targets,
    geometry, the free room of wgpu's memory blocks), the pressure climbs,
    and once the textures are lowered what the driver counts falls back
    under the budget (the `texture memory: pressure` lines give both).
  - With a value under what the viewer holds even without textures (512),
    the Memory section of the Performance window
    (`AURORA_DEMO_PERF=monitor-memory`) shows the budget in the warning
    colour and the overview the warning "Video memory over the driver's
    budget".
- **`AURORA_TEXTURE_SLOTS=<n>`**: the texture table holds n textures
  instead of 65,536 (`aurora-render/src/textures.rs`; the `renderer ready:`
  line of the log gives its size). Each texture known to the streamer holds
  a slot of it, from the first face that uses it until 45 s after the last
  one stopped. Used to fill the table in demo.
  - `AURORA_DEMO_TEXTURES=20000` and `AURORA_DEMO_POS="234,126"` (in front
    of the last cubes): without the variable every cube has its texture and
    the log has neither `texture table full` nor `upload refused by the
    renderer`. With `16384`, the size the table had before, the last 3,600
    cubes stay white (and the panels of the plaza lose their textures) and
    the log says so in one line or two for all of them, not one per
    texture: `texture table full (16384 slots, all holding a texture in
    use): … more textures have none, 3627 in all`.
  - Textures nothing uses any more give their slots back to a full table at
    once, before the others go without: `AURORA_DEMO_TEXTURES=6000`,
    `AURORA_DEMO_TEXTURES_CHURN=1`, `AURORA_FPS_LIMIT=500` and `6100`. The
    2,000 cubes that come back at frame 700 find the table full of the
    textures released at frame 300, due for eviction a second later: they
    all get their texture (capture after frame 1500 with
    `AURORA_DEMO_POS="160,126"`), `textures=` of `perf summary` never
    exceeds 6095 and the log has no `texture table full` line.

- **`AURORA_COLORED_AMBIENT=0|1|alt`**: overrides Preferences › Graphics ›
  Sky and environment › Colored sky ambient for this launch (`alt`: on
  one second, off the next). `1` lights with the sky's ambient colour x
  0.7, made linear, as the first PBR release did (atmosphericsFuncs.glsl
  `calcAtmosphericVarsLinear` and settings.xml `RenderSkyAmbientScale`,
  tag `Second_Life_Release#843866d-DRTVWR-559`). `0` is the Legacy
  Viewer's softened ambient (pow 0.9, x 0.57): it keeps its colour under
  a classic sky, and only its grey luminance under a linear or HDR one.
  Look at a shaded floor or wall under a tinted ambient:
  `AURORA_DEMO_SKY=1,0.8,0.9:0.6:0.5,0.3` (HDR): with `1` it takes the
  warm tint instead of grey. Under the classic
  `AURORA_DEMO_SKY=1,0.8,0.9:0.6:0.5` only the curve changes: the same
  tint, a little brighter. At a probe ambiance of 1 and above the probes
  replace the sky ambient (in that release too), so outdoors little
  changes there.

- **`AURORA_SPECULAR_AA=0|1|alt`**: overrides Preferences › Graphics ›
  Specular anti-aliasing for this launch (`alt`: on one second, off the
  next, without `AURORA_PROFILE`). It widens a highlight where the shading
  normal turns faster than a pixel can show (`specular_aa_roughness` in
  common.wgsl): the rims of small curved objects and fine relief seen
  from afar; a surface whose normal a pixel can resolve (a ball some 35
  pixels across or more, a flat face) is unchanged. Look at the rims of
  the polished spheres (`AURORA_DEMO_PBR_METAL=1`) and the far end of
  `AURORA_DEMO_SHIMMER=1`.
- **`AURORA_PBR_OCCLUSION=0|alt`**: by default the reflection of a PBR
  face (and, on skies that are not classic, its ambient light) is
  multiplied by the occlusion of its material, the red channel of its
  packed occlusion-roughness-metal map, as the Legacy Viewer's `pbrIbl`
  (`ao = orm.r`). A face without that map reads white: no change. `0`
  leaves the occlusion out, as before; `alt` leaves it out one second in
  three, without `AURORA_PROFILE`, to see the difference
  (`AURORA_DEMO_PBR_METAL=1`: the cube left of the spheres turns from dark
  with bright discs to an even mirror).
- **`AURORA_AO=legacy|gtao|alt[:<0-3>]`**: the ambient occlusion method of
  Preferences › Graphics for this launch, without changing the saved
  setting: `legacy` the Legacy Viewer's screen-space occlusion
  (ssao.wgsl), `gtao` ground-truth ambient occlusion (gtao.rs, after
  Intel's XeGTAO), `alt` the two by turns, one second each, both kept
  ready (GTAO on odd seconds). `:0` to `:3` choose GTAO's quality (low at
  half resolution, medium, high, ultra); without it, that of the settings
  (high when they choose Legacy). The occlusion still needs the setting on
  (Ambient occlusion: Low or High). To compare, `AURORA_DEMO_DEBUG=ao`
  shows the occlusion alone: the Legacy one is soft with wide halos and
  the relief of normal maps, GTAO's is sharp in the folds of the avatars,
  under objects and in corners. On classic skies the shading only lets the
  occlusion darken the ambient part of the light (adjustIrradiance), so the
  image itself changes little; on other skies it darkens the whole ambient
  light. `g_ssao` of `perf summary` is its GPU time (demo crowd of 40,
  2560 × 1369, RTX 4090: Legacy low about 0.08 to 0.15 ms, GTAO low 0.10,
  medium 0.24, high 0.31, ultra 0.57 ms).
- **`AURORA_GTAO_SPECULAR=0|alt`**: GTAO also gives each pixel its bent
  normal (the mean direction of the open part of its hemisphere, XeGTAO's
  `XE_GTAO_COMPUTE_BENT_NORMALS`), from which the shading takes a specular
  occlusion (`specular_occlusion`, common.wgsl; Jimenez et al. 2016): a
  reflection fades where the ray leaves away from the opening of a crease
  or a recess, on PBR faces (classic skies included, where the Legacy
  Viewer applies no occlusion to them) and on the glossy reflection of
  legacy materials. Below a roughness of about 0.3 it fades out, so mirrors
  keep their reflection; the lobe it tests is never narrower than about 26°,
  and its visibility and bent normal are blurred three times more than the
  occlusion (`denoise_bent`): without that, an inside corner of
  half-polished metal showed dark patches and soft waves. It comes with GTAO only: the Legacy method's
  image does not change. `0` leaves it out, as before; `alt` leaves it out
  every other second (`AURORA_DEMO_PBR_METAL=1`: the inside corner left
  of the cube, and the folds of the avatars' clothes and skin).
- **`AURORA_GTAO_TEMPORAL=0|alt`**: GTAO gathers its occlusion over the
  frames, as XeGTAO expects of the TAA it is made for (`temporal` in
  gtao.wgsl): its noise moves every frame (XeGTAO's SpatioTemporalNoise, 64
  frames), and each pixel blends with its history where its point was in
  the previous frame, through its depth and the camera's motion (there are
  no motion vectors), else at the same pixel when the depth there is its
  own (our avatar, which the camera follows as it walks). A history texel
  whose depth is not the point's is not read (an uncovered surface starts
  again from the current frame, converging in a few frames), and the
  history is clipped to the mean and deviation of the current 3 × 3
  neighbourhood, so the occlusion of a moving avatar or object does not
  trail behind it. The current frame weighs at least 0.1. A resize, a
  quality change, a photo frame or tile, and a frame after one without
  GTAO start again from the current frame, with the blur of before. `0`
  leaves it out (the noise fixed again: the fine grain in the penumbra of
  folds and under objects), `alt` leaves it out every other second. To
  compare, `AURORA_DEMO_DEBUG=ao` with `AURORA_AO=gtao:2` or `gtao:3`: the
  folds of the avatars' clothes and the shadow under the stools; with
  `AURORA_DEMO_CAMERA=drift,800,600,30` (the camera turning) and
  `AURORA_DEMO_DOUBLECLICK=walk` (our avatar walking, the camera behind
  it); `AURORA_DEMO_CAMERA=close,800,600,1` with
  `AURORA_DEMO_DOUBLECLICK=walk,1100,640` (the avatar walks before a
  camera that stays): no ring of occlusion lags behind its feet.
- **`AURORA_GTAO_NORMALS=depth|alt`**: GTAO starts from the surface normal
  of each pixel. The depth prepass draws the mesh's smooth normal beside
  the depth (`vs_depth_normal`, the shading's normal without its normal
  map) whenever GTAO is shown, and GTAO and its indirect light read it
  (`surface_normal` in gtao.wgsl), as XeGTAO takes a G-buffer's normals.
  Rebuilt from the depth around the pixel, as before, the normal was the
  flat triangle's: the occlusion changed from one triangle to the next and
  low-poly meshes showed their facets, more or less visible with the sky's
  ambient light (#993). Under the smooth normal, a triangle rises above the
  smooth tangent plane on one side, and its own samples occluded it there
  (a ring along the silhouette of the torus, a band at the avatar's neck),
  and the neighbours across a concave crease drew the mesh's edges as
  lines: a sample occludes nothing within 20° of the smooth tangent plane
  and fully from 44° above it (`above_surface`), measured from the smooth
  normal, so the occlusion does not change from one triangle to the next
  (measured from the triangle's own plane, real occluders in that band
  still showed the facets in concave folds: a chin, a chest). `depth` goes
  back to the depth's normal (the prepass then draws the depth alone, the
  image as before), `alt` every other second. The Legacy method does not
  change. To compare, `AURORA_DEMO_DEBUG=ao` with `AURORA_AO=gtao:2`: the
  back, the waist and the skirt of our avatar, and the large sphere behind
  the material panels, where the lines of the mesh's creases showed;
  `AURORA_DEMO_SKY=1,0.8,1.2` for a sky with strong ambient light.
- **`AURORA_DEBUG_MOTION=1`**: the motion vectors replace the image
  (`motion_debug.rs`, `shaders/motion_debug.wgsl`): the hue gives the
  direction a point moved on screen since the last frame (red left, cyan
  right, violet down), the brightness its length (two thirds of full
  brightness at about 8 pixels a frame), a dim grey where nothing moves
  (darker for the sky). The depth prepass writes them beside GTAO's
  normals (`depth_normal` in object.wgsl, from the last frame's matrices
  and joint palettes, `motion_history.rs`), so only while GTAO uses the
  mesh normals; elsewhere, and where nothing was drawn, the camera's
  motion alone is shown. Lip morphs and other morphs count as still. With
  `AURORA_DEMO_CAMERA=alt,640,400` capture frame 645 (the orbit: the sky
  one colour, the terrain shading through black where it crosses the
  pivot's distance); with `AURORA_DEMO_CAMERA=close,640,500,45` frame 625
  (the zoom: colours fanning out from the focus, the still objects the
  colour of the terrain behind them); frame 700 of the first is all grey.
- **`AURORA_SSIL=1|0|alt`**: Preferences › Graphics › Indirect light
  (GTAO) for this launch, without changing the saved setting; it needs the
  GTAO method. A pass after GTAO's gathers the light on a visibility
  bitmask (Therrien et al. 2023, `main_bitmask` in gtao.wgsl): each sample
  is a surface 0.35 m thick, which covers the sectors between its front
  and its back, and lights the pixel, for the sectors it is the first to
  cover, with the previous frame's lit colour where it was (the scene
  copy, reprojected): one bounce, weighted by GTAO's occlusion and added
  to the diffuse light of every surface (`screen_indirect`, common.wgsl).
  The occlusion itself stays GTAO's: `AURORA_DEMO_DEBUG=ao` shows the same
  image with the light on or off (the bitmask's own occlusion, used until
  2026-10-17, was softer in the corners and contacts). `alt` switches it
  every second, both kept ready (`AURORA_DEMO_BOUNCE=1`). `g_ssao`
  includes it (crowd of 40, 2560 × 1369, High: about 1.1 ms with it, 0.6
  without; at 1683 × 900, 0.62 to 0.66 ms with it, 0.23 to 0.31 without).
  Its strength is Preferences › Graphics › Indirect light intensity (0 to
  4, default 1.5; 1 is one physical bounce, which hardly shows);
  `AURORA_SSIL_INTENSITY=<0-8>` replaces it for a launch, to compare (at
  2 a glow starts around the foot of objects, at 4 it is grainy).
- **`AURORA_CONTACT_SHADOWS=0|1|alt|prepass|shell`**: the sun's contact shadows
  (Preferences › Graphics › Contact shadows, `contact_shadows.rs`, after
  Bend Studio's screen space shadows) for this launch, without changing the
  saved setting: `0` off, `1` on, `alt` off and on by turns, one second
  each (on in odd seconds), the mask kept ready; `prepass` on, but cast by
  the depth prepass alone (opaque and masked faces, terrain), as before the
  blended faces opaque enough and the impostor cards were drawn into a copy
  of it for them (see `AURORA_DEMO_FEET`); `shell` on, each depth taken
  as a shell of 0.5 % of its distance (Bend's window, before issue 988)
  instead of a body as deep as the ray went under it (up to 5 cm, with
  the sun ahead of the camera): a ray that goes under a thick caster is
  stopped only where it grazes its edges. **`AURORA_CONTACT_SOFT=0`**:
  the mask reaches the shading as the pass wrote it, without the 5 × 5
  softening over each surface (`contact_shadows_filter.wgsl`): hard steps
  of whole pixels on curved bodies (the sides of our avatar with
  `AURORA_DEMO_SUN=0,12`) and a dotted line under the hovering shoe of
  `AURORA_DEMO_FEET` seen from above (`AURORA_DEMO_CAMERA=alt,615,672`,
  frame 690). Softened, neither shows and no shadow spreads from the
  ground onto a caster nor the reverse.
  **`AURORA_CONTACT_VARIANT=no-early-out|no-shared|short|skip`**: a
  diagnostic of the GPU hangs (device lost, Windows TDR) some AMD RX 9070 XT
  cards had as soon as the pass ran, each value without one construct of
  the window's shader (`variant_source`, contact_shadows.rs; photos keep
  Bend's): `no-early-out`, no early exit of an all-sky wavefront through
  `workgroupUniformLoad`; `no-shared`, no workgroup memory nor barrier (each
  thread reads from the depth what the others would have shared,
  `contact_shadows_no_shared.wgsl`). Both draw the same mask as Bend's
  (the GPU test `the_variants_that_keep_the_image_draw_bends_mask`, run
  with `-- --ignored`). `short`: 16 samples per pixel instead of 60, the
  shadows about four times shorter; `skip`: the shader is not dispatched,
  no contact shadow (the mask is still cleared and filtered). Any other
  value keeps Bend's shader; the log says `contact shadows: diagnostic
  variant …` when one is taken. They only draw with the
  sun shadows on. To see them, `AURORA_DEBUG_SHADOWS=1` (the shadow view
  takes them too) with `AURORA_DEMO_TOD=2` (high sun) or `3` (low sun),
  `AURORA_DEMO_CROWD=40` and `AURORA_DEMO_POS=129,134`: thin shadows appear
  where things touch or overhang closely — arms against the body, the
  waist over the skirt, the base of the lamp posts, the crowd's hands —
  which the shadow map is too coarse to hold. `g_contact` of
  `perf summary` is their GPU time, the copy of the depth and its extra
  casters included (same crowd with `AURORA_DEMO_FEET=blend`,
  2560 × 1369, RTX 4090, 120 fps cap: about 0.17 to 0.26 ms).
- **`AURORA_BLOOM=0|1|alt`**: Aurora's bloom (Preferences › Graphics ›
  Bloom, off by default) for this launch, without changing the saved
  setting: `0` off, `1` on (at the setting's strength, 0.5 when it is off),
  `alt` on one second in two, its levels kept ready. Only what comes close
  to white blooms (sun on metal, lights, fullbright and emissive faces); the
  glow of faces with a glow value is the Legacy Viewer's and does not
  change. To look at: `AURORA_DEMO_PBR_METAL=1`,
  `AURORA_DEMO_CAMERA=close,675,455,30` (frame 760 at 60 fps): the
  highlights of the polished sphere gain a soft halo, the rest of the image
  stays as it is. Off, the post pass reads the scene exactly as before (two
  captures off differ by the demo's run-to-run noise only). `g_bloom` of
  `perf summary` is its GPU time (demo, 2560 × 1369, RTX 4090: about
  0.07 ms on the plaza, 0.09 ms in a crowd of 40).
- **`AURORA_AUTO_EXPOSURE=0|1|alt`**: the automatic exposure of skies that
  are not classic (reflection probe ambiance above 0), like the Legacy
  Viewer's dynamic exposure, for this launch: `0` off (exposure 1 on every
  sky, as before), `1` on (the default), `alt` on one second in two, switched
  at once without the easing. The middle of the image (the luminance of the
  ground counted at half) sets an exposure between `1 / (sqrt(gamma) x 2)`
  and `sqrt(gamma) x 2` of the sky, eased towards over about two seconds:
  a dark scene opens up, a bright one closes down; a classic sky keeps 1.
  The log gives the value every 30 s and 4 s after a change of sky:
  `auto exposure: 1.805 (target 1.809, average luminance 0.178, range 0.500
  to 2.000)`. Demo: `AURORA_DEMO_SKY=1,0.8,0,1` (a non-classic sky of gamma
  1, like the library Midday) gives that line; `AURORA_DEMO_SKY=1,0.8,1,1`
  (ambient 1) a brighter scene and a lower exposure (about 1.6).
- **`AURORA_PROBE_WALK=list|0|alt|check`**: by default a fragment of the
  main view only walks the reflection probes that cover at once its screen
  column, its row and its depth slice (the sets of the lights,
  `AURORA_LIGHT_TILES`, filled for the probes: `probes::tiles` in
  `aurora-render/src/probes.rs`), once for its two lookups (ambient and
  reflection of a PBR face, glossy and environment reflection of a legacy
  one; `sample_probes_pair` in `common.wgsl`). Same image: a probe left out
  covers none of the fragments drawn there, and those that stay are added
  up in the order of the list. Water finds its probes the same way.
  - `list`: one walk of the whole list (32 probes by default) for the two
    lookups, as before the sort, to compare. Reflections, probe faces and
    impostor photos do not sample the probes.
  - `0`: one walk per lookup, each going through the whole list twice, as
    at first.
  - `alt` (with `AURORA_PROFILE=1`): the whole list during one
    `perf summary` line out of three (`xmode=old`), the sort during the
    other two (`xmode=new`). Compare `g_objects`, `g_transparent` and
    `gpu`. In demo (`AURORA_DEMO_CROWD=40`, `AURORA_DEMO_POS="135,134"`,
    `AURORA_WINDOW_SIZE="2560,1369"`, `AURORA_FPS_LIMIT=120`, 16 probes):
    `g_objects` 0.68 → 0.58 ms, as much as without any probe but the
    default one (0.60 ms): the walk was the cost, not the samples.
  - `check`: the sort, and any fragment that a probe covers without being
    in its sets is painted magenta (on the faces that read the probes: PBR
    ones, shiny legacy ones). There must be none, whatever the camera does;
    try it with `AURORA_DEMO_PROBES=1` and `AURORA_DEMO_PBR_OVERRIDE=1`,
    under MSAA too (`AURORA_AA=3`): a sample at the edge of a triangle seen
    edge-on is shaded at a position that can be behind the camera, which is
    why such positions walk the whole list.
- **`AURORA_LIGHT_TILES=0|alt|check`**: by default, a lit fragment only
  walks the local lights that cover at once its screen column, its row and
  its depth slice (32 of each; sets computed each frame on the render
  thread, `aurora-render/src/light_tiles.rs`), in the order of the list:
  same image, the lights that reach the fragment are added up as before and
  in the same order.
  - `0`: each fragment walks the whole list (up to 64 lights), as before,
    to compare.
  - `alt` (with `AURORA_PROFILE=1`): the old walk during one `perf summary`
    line out of three, the new one during the other two, in the same
    session and on the same scene; `light_tiles=1.00` or `0.00` in the line
    tells which (an intermediate value: the second of the change). Compare
    `g_objects`, `g_terrain_sky` and `g_transparent` + `g_post` of the two
    groups of lines, with `AURORA_FPS_LIMIT=120`; `lights=` gives the
    length of the list.
  - `check`: the shaders paint in magenta any fragment that a light of the
    list reaches without being in its sets. There must be none, wherever
    one looks (expensive: each fragment walks the whole list again).
  - Reflections, probe faces and impostor photos keep the whole list.
  - In demo: `AURORA_DEMO_LIGHTS=64` with `AURORA_DEMO_CROWD=40`.
- **`AURORA_BLEND_EARLY_DISCARD=0|alt`**: by default, a fragment of a
  transparent face whose alpha is below 0.004 is discarded before lighting
  (shadows, lights, haze, probes); it used to be discarded after.
  Firestorm's `alphaF.glsl` likewise discards it before the haze, the
  probes and the lights. Same image. `0`: the old order, to compare; `alt`:
  the old order during one `perf summary` line out of three (`blend_early=`
  in the line), like `AURORA_LIGHT_TILES=alt` — alternate only one of the
  two variables at a time, they change at the same seconds. The gain
  depends on the content: see `AURORA_DEMO_HAIR`.
- **`AURORA_RIGGED_BLEND=split|runs|copy|depth05|write|sorted|alt`**: by
  default the
  blended faces of rigged meshes (mesh hair, eyelashes, clothes, animesh)
  are drawn before the other blended faces, skeleton by skeleton back to
  front, each skeleton's in the Legacy Viewer's order: the last attachment
  point first, on one point the last attached first (by the local id of
  the linkset's root), the prims of a linkset and the faces of a prim in
  turn, the triangles of a mesh face in the Legacy Viewer's order
  (`AURORA_MESH_ORDER`). Object by object, on a copy of the scene depth,
  their texels at least half
  covered are drawn first, each writing its depth as it is drawn, as the
  Legacy Viewer's alpha pass does from 1/255; then their fainter texels,
  depth tested; then the depth of those fainter texels, from 1/255. An
  object hides what is drawn after it and behind it wherever it has a
  texel, as in the Legacy Viewer (a sheer dress hides a body add-on drawn
  after it, #1300, `AURORA_DEMO_SHEER`), but none of its own strands
  behind a soft edge: a hair made inner
  layer first composes as its creator saw it, an inner layer drawn after
  the outer one no longer shows over it (#935: dark patches the shape of
  its triangles with `runs`, `AURORA_DEMO_HAIR_CARDS=<n>,layered,outer`),
  the cards of one face leave no shards, and the scene depth then takes
  them only where they are at least half covered, so that their nearly
  transparent fringe does not
  hide the glass, water and particles drawn after them (#556). `split`: as
  #1024, run by run (a new run for each mesh worn on a point that already
  has one in the run), on the scene depth, the fainter texels writing no
  depth: they hide nothing drawn after them, so a mesh drawn after a sheer
  dress and under it shows through it in speckles wherever the dress is
  under one half (#1300), while the fringes of hair blend over a body
  drawn after it. `runs`: as
  #966, their depth written where they are nearly opaque (95 %) before any
  of their colour; then, attachment point by attachment point, their
  colour drawn depth tested and their depth written where at least half
  covered: a strand between 50 and 95 % hides nothing drawn after it.
  `copy`: as
  #948, their colour writing its depth from 1/255 as it is drawn, as the
  Legacy Viewer's alpha pass (`LLDrawPoolAlpha::forwardRender(true)`), on a
  copy of the scene depth, then where half covered into the scene depth: a
  soft strand edge hides the strands drawn after it behind it, and the
  cards of one face, in no back to front order, leave shards where the
  scene shows through the hair (`AURORA_DEMO_HAIR_CARDS`). `depth05`: as
  before #948, their depth where at least half covered written before
  their colour, which is depth tested without writing it: a layer drawn
  first is hidden behind every strand of the layers over it from 50 % on,
  which then shows what lies behind the hair through it (the strands of the
  outer layer look see-through, with stair steps where they cross 50 %).
  `write`: their depth written from 1/255 into the scene depth, as the
  Legacy Viewer (#499): a ring around fur or hair where the glass behind is
  missing. `sorted`: everything sorted by the distance of its object and
  nothing writing depth (eyelashes showed through mesh hair). `alt`:
  `split` during one `perf summary` line out of three (`xmode=old`). See
  `AURORA_DEMO_HAIR`, `AURORA_DEMO_HAIR_CARDS`, `AURORA_DEMO_LASHES`,
  `AURORA_DEMO_SHEER` and `AURORA_DEMO_HALO`. Cost, `AURORA_DEMO_CROWD=40`
  with `AURORA_DEMO_HAIR=8` (eight meshes on one point per avatar, one run
  each) at 120 fps: `g_transparent` about 0.21 ms with `runs`, 0.50 ms with
  `split`; with `AURORA_DEMO_HAIR=1` (one mesh per avatar), 0.076 and
  0.082 ms. By default, against `split` (60 fps cap, `alt`): 0.52 → 0.66 ms
  with eight meshes each, 0.083 → 0.131 ms with one.
- **`AURORA_MESH_ORDER=asset`**: the triangles of every mesh face are drawn
  in the order the Legacy Viewer draws them, reordered for the vertex cache
  when the mesh is loaded (`LLVolumeFace::cacheOptimize`,
  `meshopt_optimizeVertexCache`): without a depth sort, the strand cards of
  a blended mesh hair blend and hide one another in that order, the one
  its creator saw. `asset` keeps the order of the asset, as before, to
  compare.
- **`AURORA_BLEND_AO=new|none|old|alt`**: the occlusion (both methods)
  reads a copy of the depth prepass with the blended faces at least half
  covered drawn over it (`fs_depth_contact`, the rule of the contact
  shadows), and those faces take the screen-space ambient occlusion and
  GTAO's indirect light like the opaque ones (#882: a mesh head whose two
  halves differ in alpha mode showed a line down the middle with the
  occlusion on); more transparent ones take neither, as the Legacy
  Viewer's alpha pass. `none`: no blended face takes them and the
  occlusion reads the prepass alone (#556 to #882); `old`: every blended
  face takes those of what lies behind it (before #556); `alt`: `none` one
  second in two. With the contact shadows on, their pass copies that depth
  instead of drawing the blended faces a second time. See
  `AURORA_DEMO_BLEND_AO`.
- **`AURORA_DEMO_HALO=1`**: a wide translucent pane (35 % opaque) 3 m
  ahead of the start position (`AURORA_DEMO_POS`), facing the camera, so
  that our avatar stands between the camera and the pane. With
  `AURORA_DEMO_HAIR=8,24,60,16` and `AURORA_RIGGED_BLEND=write`, the ring
  of #556 (the pane missing around the hair) appears; by default it does
  not.
- **`AURORA_DEMO_BLEND_AO=1`**: a flat face 4 m ahead of the start
  position (`AURORA_DEMO_POS`) and 1.6 m to its left, above our avatar's
  head, turned towards the camera: two halves alike, the left one blended
  (alpha 0.99, opaque to the eye), a ball for a nose across the middle, a
  wide hat brim above and two curtains of hair on its sides. With
  `AURORA_DEMO_CAMERA=close,664,452,40`, a capture at frame 780 and
  `AURORA_DEMO_DEBUG=ao`, `AURORA_BLEND_AO=none` shows the occlusion
  around the nose stopping at the middle line, the left half white;
  the default shows it whole (#882).
- **`AURORA_FONT_FALLBACK=0`**: uses only the embedded fonts, as before the
  system fallback fonts were added: the characters they lack (Georgian,
  CJK, Arabic, mathematical letters…) become squares again. To compare with
  `AURORA_DEMO_GLYPHS=1` or to isolate a font problem. The log says
  `system fallback fonts: off`.
- **`AURORA_UI_FONT=<font>`, `AURORA_UI_SCALE=<scale>`**: the interface
  font (`Inter`, `Noto Sans`, `Roboto`, `OpenDyslexic` or
  `system:<path of a .ttf>`) and
  scale (0.75 to 2) of this launch, without touching Preferences ›
  Fonts / Interface: to capture one screen with every font and scale in a
  row. Give the window the scale too (`AURORA_WINDOW_SIZE="2304,1296"` at
  1.5), else the top bar drops what does not fit. What to look for: text
  beside an icon, a badge or a box (the place island's rating badge and
  name, buttons, menu rows) centred alike with every font; the test
  `the_place_island_centres_its_badge_and_texts_with_every_font` measures
  the place island.
- **`AURORA_TEXT_SIZE=<size>`**: the text size (0.8 to 2, Preferences ›
  Appearance › Text size) of this launch, never saved: to capture any
  scenario at a large text size, for example the Conversations window and
  the inventory at 2 (`AURORA_DEMO_INVENTORY=1`, 1920 × 1080). What to look
  for: no line drawn over another (a conversation's name and the line under
  it, a sidebar label and its count, a footer's hint and its buttons),
  initials and the AO glyph inside their tile like the icons beside them,
  windows that keep their size.
- **`AURORA_PRESENT_MODE=immediate|mailbox|fifo|fifo-relaxed`**: replaces
  the present mode chosen from vertical sync
  (`aurora-render/src/present.rs`: without sync `Mailbox`, otherwise
  `Immediate`; with it, `FifoRelaxed`, otherwise `Fifo`), if the surface
  offers it; an unknown value is reported in the log and ignored. Used to
  compare the fps from one screen to another (screens with different
  refresh rates, variable refresh rate G-Sync / FreeSync): `immediate`
  never synchronises with the screen (tearing possible), `fifo` aligns
  with its refresh rate. The log gives the mode retained and those of the
  surface (`presentation: Mailbox (vsync off, surface supports […])`), then
  the screen under the window and its refresh rate at each change
  (`screen under the window: \\.\DISPLAY2, 144 Hz`); `present=` and
  `screen_hz=` appear in the `perf settings` line.
- **`AURORA_TEXTURE_BC7_CACHE=0`**: the textures compressed to BC7 are
  neither written to the disk cache (`cache\tex\<id>.bc7`) nor read back:
  each texture is decoded from its JPEG2000 as on a first visit, to measure
  what the cache saves (`read as BC7 without a decode` and the CPU time in
  the `texture load:` line).
- **`AURORA_TEXTURE_BC7=0`**: all the textures stay in RGBA8 in video
  memory, as before BC7 compression, to compare the image, the memory
  (`texture_mb`, `pages_mb` of `perf summary`) and the loading time (`bc7`
  in the `texture load:` line). By default, when the graphics card has BC
  compression, each decoding task encodes the texture to BC7 and checks the
  result; those it would damage stay in RGBA8. The log announces it at
  startup (`textures: BC7 compression on|off (GPU support: …)`).
- **`AURORA_CPU_CULL=1`**: builds all the draw lists on the CPU (the
  fallback when the GPU does not have `MULTI_DRAW_INDIRECT_COUNT`) instead
  of sorting them on the GPU, to compare; `cull=cpu|gpu` in the
  `perf summary` line.
- **`AURORA_WINDOW_HOLD=0`** (Windows): by default, the viewer keeps
  drawing while Windows holds its window: a mouse button pressed on the
  title bar, a border or the close / maximize / minimize button, a move or
  a resize, the system menu open. Windows follows those in a message loop
  of its own, in which the usual request for the next frame is never made;
  a helper thread (`window_hold.rs`) asks for the frames for as long as the
  hold lasts, and sleeps the rest of the time. During a hold the frame
  limiter does not wait on the main thread, which Windows needs for its
  loop: the helper waits between two frames, and spaces them out (60 a
  second at most) while the cursor moves, so that a dragged or resized
  window keeps following the cursor.
  - `0`: no helper, as before: the image stays still for the whole hold,
    which `perf summary` shows as one long `between` in `max:`.
  - It needs real mouse input, there is no demo scenario: hold the button
    for a second or two on each of those spots with `AURORA_PROFILE=1` and
    read `max=` and `between=` of the second (a hold of 1.5 s: 1,500 ms
    with `0`, 20 to 30 ms at most without). The buttons must still act
    when released on them, and not when released elsewhere; a window
    dragged or resized with fast mouse moves must stay under the cursor,
    as with `0`.
- **`AURORA_TEXTURE_TODO=0|check`**: by default, the texture streamer
  (`TextureStreamer::update`) only visits at each frame the textures that
  have work: a first disk read, or a finer level than the one on the GPU
  to read, decode or download. A texture is listed when it gets some, and
  the counts shown by the performance window (loaded, downloading,
  failing) are kept as the textures change. Same loading, same image.
  - `0`: every texture is walked at each frame, as before, to compare
    `s_tex_update` in `perf summary`.
  - `check`: one frame in thirty, the list and the counts are compared
    with a walk of every texture; the log gives the differences
    (`texture todo check`). There must be none, in demo mode as on the
    grid.
  - The `streaming:` line of the log, every 30 s, gives the textures
    listed (`with work`): a few at rest (those that never load), all of
    them while a region arrives.
  - In demo mode: `AURORA_DEMO_STREAM=6000,6000` with
    `AURORA_FPS_LIMIT=240` (6,000 textures kept loaded), with `,leave`
    (released then evicted), and with `AURORA_TEXTURE_BUDGET_MB=96` and
    `AURORA_DEMO_POS="168,126"` (lowered under memory pressure, then
    loaded again).
- **`AURORA_LIST_INDEX=0|check`**: by default, the draw lists and those of
  the HUDs find the avatars, the objects with transparent or glowing faces,
  the slice of the texture tracking and the worn HUDs in one byte per
  object slot (`scene/list_index.rs`), kept up to date where the GPU's
  object table is. Same image.
  - `0`: they walk the objects themselves each frame, as before, to
    compare `lists` in `perf summary`. The gap grows with the largest
    number of objects seen since the start of the session, not with those
    that are there.
  - `check`: one frame out of thirty, the index is compared with the
    objects; the log gives the slots that differ (`list index check`).
    There must be none, in demo as on the grid.
  - In demo: `AURORA_DEMO_CROWD=52,200` with `AURORA_DEMO_ANIMESH=1` and
    `AURORA_DEMO_STREAM=6000,6000,leave` (19,500 objects, then 13,500),
    `AURORA_FPS_LIMIT=500`.
- **`AURORA_WORN_RANK=own`**: by default anything worn by another avatar,
  and the bakes of an avatar still loading, rank in the download queue
  with the avatar's radius (2.2 m) over its distance, as the Legacy
  Viewer's `calculate_score` (llmeshrepository.cpp) and
  `addBakedTextureStats`. With `own`, a worn mesh ranks by its own prim
  scale until its header says it is rigged, as before (the bakes keep the
  avatar's rank). The avatars come sooner, the scenery's textures start a
  little later. To compare with
  `AURORA_DEMO_ARRIVAL=<folder>,avatars=30,worn=6` (`demo arrival: worn`
  line), or on the grid in a crowd with a cold cache (the `attachments of`
  blocks 40 s after the arrival: `without geometry`).
- **`AURORA_CPU_ATTACH=1`**: places again each frame, on the main thread,
  everything the avatars wear and all the rigged meshes, as before the
  anchors (`scene/anchors.rs`), to compare the image and the measurements.
  By default, an object worn on the body is placed once relative to its
  attachment point, a rigged mesh (worn, bento, animesh) and an avatar's
  body relative to their skeleton, and the GPU does the rest, culling
  spheres of the rigged meshes included; the HUDs, the flexible prims and
  the objects that rotate by themselves stay with the sync. To read in
  `perf summary`: `anchored` (objects placed by the GPU, 0 with the
  variable), `synced`, `sync` and its parts `s_sync_*`, `records_kb`,
  `journal_kb`. To be combined with `AURORA_DEMO_CROWD=<n>` and
  `AURORA_DEMO_ANIMESH=1`. Only expected difference in the draws: the
  transparent faces of the rigged meshes, which the CPU lists, are tested
  with the extent of their skeleton and no longer with their own bounds
  (`blend` slightly higher).
- **`AURORA_MATERIAL_REBUILD=all`**: each material that arrives (or is given
  up) rebuilds every object using materials, as before, instead of the users
  of that material only (`scene/mat_changes.rs`). `alt`: the old way one second
  in three, `xmode=old` / `xmode=new` at the end of the `perf summary` line.
  With `AURORA_DEMO_MATERIAL_ARRIVALS`, the old seconds show the bursts of full
  syncs (`rebuilt`, `max: sync`).
- **`AURORA_SYNC_SLICE=1`**: the scene sync comes back to each object every
  24 frames (LOD slice), places the worn HUDs again each frame and sends
  the records in whole blocks of 64, as before, to compare the image and
  the measurements. By default, a still object is only revisited when the
  camera has travelled the distance that can change its level of detail
  (`scene/lod_watch.rs`: never with a still camera); what the GPU places
  from an avatar (worn objects, rigged meshes) is revisited in turn, every
  24 frames while the camera moves (as before) and every 0.4 s with a still
  camera, and every object every 10 s; a worn HUD is only placed again on
  its updates and when its place on screen changes (window resized, zoom or
  local offset); only the modified records are sent.
  - `AURORA_SYNC_SLICE=alt` alternates within the same session, at each
    `perf summary` line: one second by the old path, two by the new one,
    the line ending with `xmode=old` or `xmode=new`. This is the
    measurement to make on a shared machine or on the grid: same objects,
    same camera, same outside load on both sides.
  - To read: `synced`, `sync` and its parts `s_sync_*`, `records_kb`,
    `cull_kb`, `journal_kb`, `r_resources`. To be combined with
    `AURORA_DEMO_CROWD=40,busy`, `AURORA_DEMO_ANIMESH=1` and
    `AURORA_DEMO_STREAM=12000,400`.
- **`AURORA_PENDING_PARK=0`**: the objects whose geometry is not bound yet
  are all asked about again by the sync, 2,048 a frame at most, as before.
  By default those waiting for a geometry still being built or downloaded
  leave that round until it is ready or dropped, and come back to it every
  2 s in case (`scene/sync_sets.rs`). Compare `s_sync_list` over whole
  runs with many meshes still downloading: `AURORA_DEMO_ARRIVAL=<folder>,
  net=200,bw=8,n=6000,avatars=30,worn=6` (about 2,000 geometries pending
  for a minute).
- **`AURORA_SYNC_CHECK=1`**: one `sync check` line per second, in averages
  per frame: the objects listed by the sync according to what put them
  there (`moving`: what moves by itself and what follows it; `changed`:
  updates; `geometry`: geometry arrived; `lod`: checks of the level of
  detail), then by kind of object (`avatar`, `hud`, `flexible`, `moving` —
  velocity or rotation given by the simulator —, `rigged` — rigged mesh
  that the GPU does not place —, `anchored` — placed by the GPU —, `worn` —
  worn and placed by the sync —, `follower` — linked under an object that
  moves —, `still`) what came out of it: placement verified without writing
  anything / records rewritten / full sync / full sync postponed
  (`scene/sync_check.rs`). Tells which objects keep the sync busy in a
  place at rest. The line ends with a check of all the objects, a slice
  per frame (a whole pass in at most 120 frames), the last full pass:
  `detail level behind: a of b at rest, c of d placed by the GPU` counts
  those whose requested level of detail is not, at that instant, the one
  their distance gives (a check the sync has not made yet); to be read with
  the camera moving, with and without `AURORA_SYNC_SLICE=1`. Made at once,
  that pass cost 5 ms in one frame of each second at KB: do not read the
  frame times of a run with this variable. Every 10 s, an `impostor
  pictures` line counts the pictures taken with the pose of an earlier
  frame and the centres of worn objects left as they were that are not
  where their anchor puts them (both must stay 0; a picture or two in the
  first seconds, before the palette exists, is the same in both ways of
  `AURORA_IMPOSTOR_POSES`). In demo as on the grid.
- **`AURORA_SYNC_SLOW=<ms>`** (8 if not a number): one `slow sync` line for
  each frame whose scene sync took longer than `ms`: the time of each step
  (`garbage`, `list`, `plan`, `apply`, `arrivals`, `backlog`, `check`,
  `anchors`, `flexibles`, `tables`, `geom_gc`, `alpha`…), the moves, the
  full syncs by kind with their time (`avatars`, `listed` for new
  geometry or faces, `arrivals`, `backlog`) and the slowest object of the
  frame (`scene/sync_slow.rs`). The `perf summary` gives the slowest frame
  of each second by part; this line names the step and the object. Cheap
  enough for a grid reading.
- **`AURORA_IMPOSTOR_POSES=every`**: poses the avatars drawn as impostors
  at every frame, as before. By default an impostor keeps its last pose
  until its picture is nearly due (`scene/impostors.rs`), as the Legacy
  Viewer animates an impostor only on the frames that may refresh it; its
  animations still start and stop on time, and the anchors of what it
  wears are not written again while it stands. `alt`: the old way one
  second in three, the same seconds as `AURORA_MATERIAL_REBUILD=alt`
  (`xmode=old` / `xmode=new`); read `posed`, `poses` and `s_sync_anchors`.
  Its old seconds also write every anchor and every centre again, as
  `AURORA_ANCHOR_CENTRES=every`: with `AURORA_MATERIAL_REBUILD=alt` too,
  they are the sync of before this work, to tell whether a fault on the
  grid comes from it (it then shows one second in three only).
- **`AURORA_ANCHOR_CENTRES=every`**: recomputes the centre of every object
  placed from an anchor at every frame, as before, instead of those whose
  anchor changed, and writes again the anchors of every wearer
  (`scene/anchors.rs`).
- **`AURORA_RENDER_THREAD=0`**: draws the frames on the main thread, as
  before the render thread, to compare and debug. By default, the main
  thread hands each frame to the render thread as a self-contained packet
  and moves on to the next frame; with `0`, the same packet is executed in
  place. `thread=on|off` in the `perf summary` line, and a log line at
  startup (`renderer: frames drawn by…`).
- **`AURORA_FRAME_PACING=0`**: turns off the frame pacing, to compare.
  By default, once it has handed frame N over, the main thread waits, at
  the place of the frame limiter and before the next frame reads its
  input, until the GPU has finished frame N−1: when the GPU (or vsync)
  sets the pace, the next frame reads its input while the GPU draws frame
  N, at the same frame rate. Without it, the input of a frame waits three
  or four frames for the screen (windows dragged behind the cursor, a slow
  camera, #1484). The wait is on only while the GPU's time covers the
  main and render threads' together (smoothed over about ten frames, 5 %
  margins): with the three stages close, a busy region on a fast card or
  two viewers on one card, waiting cost a quarter of the frame rate
  (#1711), so the frames overlap there as without pacing. Without GPU
  timestamps it stays off. `pace` (that wait, 0 while it is off) and
  `input` (input to presentation) in the `perf summary` line; with `0`,
  `pace` stays at 0 (`aurora-render/src/pacing.rs`). `always` waits on
  every frame, whoever sets the pace, as 0.8.4 did: the "before" of a
  comparison in a busy region.
- **`AURORA_FRAME_PRIORITY=normal|alt`**: by default the threads a frame
  waits for (the main thread, the render thread, the pool workers that
  help the main thread in the frame's parallel loops and the texture page
  thread) run one step above normal priority, so that other programs
  keeping the cores busy do not take a core from a step of the frame
  (`aurora-render/src/frame_priority.rs`). `normal` keeps them at normal
  priority, as before; `alt` puts them back at normal priority one second
  in three, named `xmode=old` at the end of the `perf summary` line
  (`xmode=new` for the others): read `ft=` and `off_cpu:` per mode, on a
  machine that is busy with something else (a build, another viewer).
  Windows only: on Linux the frame's threads are not raised, and both
  modes are the same (the background jobs still run 10 nice values above
  the viewer's).
- **`AURORA_ALLOCATOR=system`**: the process allocates with the system's
  allocator (the Windows process heap), as before, instead of mimalloc
  (`allocator.rs`). The choice holds for the whole run, read before
  `main`; the log's first line says which one runs (`starting (windows
  x86_64, mimalloc allocator)`). The system heap has one lock for every
  thread: when the background jobs decode and build in a burst (the camera
  turning in a dense place, an arrival), the main thread waits on it at
  its allocations and frees, a frame of 25 to 110 ms with most of it off
  the processor. To compare, alternate runs of both on a load scene
  (`AURORA_DEMO_ARRIVAL=<folder>,net=20,n=12000,avatars=60,worn=8`) and read
  the worst frame of each second (`max=`) and its `off_cpu:` segment.
- **`AURORA_MSAA_PREPASS=0|alt`**: with MSAA, the scene pass has its own
  multisampled depth and cannot reuse that of the single-sample prepass. By
  default, a second prepass fills this depth just before (terrain and
  opaque faces, the very draws of the scene pass), and the scene pass no
  longer computes the colour of the hidden fragments. The alpha-masked
  faces are not in it: with MSAA their edge comes from the
  `alpha to coverage` of the scene pass, which a depth pass cannot
  reproduce sample by sample.
  - `0`: the scene pass starts from an empty depth, as before, to compare
    the image and the times (`g_objects`, `g_terrain_sky`, `g_depth_msaa`
    of `perf summary`).
  - `alt`: one second without the prepass, two seconds with it, throughout
    the session; at each switch the log gives the median of the GPU times
    of the frames of the variant that ends
    (`msaa prepass alt: prepass=on|off frames=… gpu=… shadows_depth=… …
    objects=… depth=… depth_msaa=…`, in ms). This is the measurement to use
    on a shared card or on the grid: both variants see the same scene and
    the same load, and the median discards the turn of another process on
    the card. Compare the medians of the `on` and `off` lines, preferably
    with `AURORA_FPS_LIMIT=120`.
  - No effect outside MSAA (`AURORA_AA` 0, 1, 5, 6): with `alt`, the `on`
    and `off` lines then measure the same path twice, which gives the gap
    between two equal measurements.
  - To be seen with `AURORA_DEMO_OVERDRAW=12` (clear gain) and with the
    demo alone (little overlap: the prepass costs about what it saves).
- **`AURORA_EYE_RELATIVE=0`**: computes the vertices of the objects and the
  terrain in region coordinates, as before, to compare. By default, the
  vertex shader removes the camera position from the model's translation
  before any other computation (`eye_model` in `object.wgsl`,
  `eye_view_proj` matrix): at altitude, single precision only holds 0.24 mm
  in region coordinates (between 2,048 and 4,096 m), and each vertex of the
  nearby objects fell up to one pixel off, somewhere else at each change of
  view. To be seen with `AURORA_DEMO_ALTITUDE=2085` and
  `AURORA_DEMO_CAMERA=drift,690,420,84` on consecutive screenshots (frames
  800 to 899): the edge of the panel drifts steadily, or jitters with `0`.
- **`AURORA_SHADOW_LEGACY=1|splits|separate|ab|merge`**: to compare the
  image and the measurements of the shadows. By default, each cascade only
  draws the objects whose shadow can fall on what the view reads in it
  (`aurora-render/src/shadow_cull.rs`), the opaque faces are drawn without
  a fragment shader, the cascade of a pixel is chosen from its own distance
  to the camera, and an opaque object that the cascade before already holds
  is not drawn again: that part of the cascade is filled from the finer
  map (`aurora-render/src/shadow_merge.rs`), so most objects are drawn in
  one cascade only.
  - `separate`: each cascade draws all its objects, as before. An object
    whose shadow reaches two or three cascades is drawn in each: in a
    crowd, every avatar.
  - `merge`: with `AURORA_PROFILE=1`, `separate` one second in three and
    the default path the other two (`shadow_path=separate|new`). In the
    demo crowd (`AURORA_DEMO_CROWD=40`, `AURORA_DEMO_ANIMESH=1`,
    `AURORA_DEMO_ANIMESH_TRIS=200000`, `AURORA_FPS_LIMIT=500`): about half
    the shadow triangles (`st_k0` + `st_k1` + `st_k2`), at noon as at
    sunset (`AURORA_DEMO_TOD=3`). Nothing must be seen changing from one
    second to the next.
  - Expected image difference with `separate`: none but the place of a
    shadow edge within a texel of its cascade (the finer map is read at the
    centre of each texel of the coarser one, where a draw would have
    sampled the object). The alpha-tested faces are drawn in every cascade
    as before, since a finer map cuts their texture out differently.
  - `1`: the old path. All the objects of each cascade's square, all the
    faces with the alpha test, and the cascade chosen from the distance
    interpolated between the vertices of the triangle.
  - `splits`: only the old choice of the cascade. The image is then that of
    the old path to the pixel; the objects close to the camera stay drawn
    in the far cascades.
  - `ab`: with `AURORA_PROFILE=1`, the old path, the default one and `splits`
    in turn, one second each, in the same session and under the same
    outside load. `shadow_path=legacy|new|splits` in the `perf summary`
    line tells which one drew the second; compare `shadow_draws`,
    `g_shadows_depth` and `gpu`, with `AURORA_FPS_LIMIT=120` for the GPU
    times.
  - Only expected image difference with `1`: on a triangle that covers
    several cascades (large floor, large wall), the boundary between two
    cascades moves, and the shadows close to the camera are sharper there
    (they were reading a farther cascade than the terrain and the small
    neighbouring objects).
- **`AURORA_TANGENTS=vertex|pixel|derivative|alt`**: the tangent frame of
  the normal maps (`perturb_normal`, common.wgsl). By default (`vertex`) it
  is the vertices' tangents, built with the geometry as the Legacy Viewer
  builds them (`scene/tangents.rs`: Lengyel's for prims, MikkTSpace for
  meshes), interpolated across the triangles; faces without them (avatar
  bodies, terrain) take the per-pixel frame. `pixel` uses only the
  per-pixel frame: T along dP/du of each triangle, of unit length and
  orthogonal to the normal, B = cross(N, T) with the handedness, built from
  the derivatives (not smoothed across the triangles' edges). `derivative`
  brings back the cotangent frame of before, scaled by the longer of its
  two gradients, which divided the relief along the denser texture axis by
  the density ratio; `alt` alternates `vertex` and `pixel` every second.
  See `AURORA_DEMO_BUMP`.
- **`AURORA_ALPHA_COVERAGE=0`**: the mips of a texture whose alpha is a
  mask are box-filtered as they come (`build_mips`), so a leaf, a blade of
  grass, a strand or a fence wire falls under the alpha cutoff a few levels
  down and the foliage thins out with the distance. By default each level
  keeps level 0's share of texels at or above the 0.5 cutoff
  (`preserve_alpha_coverage`, Castaño 2010), computed once at decode and
  stored in the transcoded cache (a file made the other way is decoded
  again). It is read at start: compare two launches. See
  `AURORA_DEMO_FOLIAGE`.
- **`AURORA_MIP_FILTER=weighted`**: the colour of each mip texel is the
  average of its four texels weighted by their alpha, as Aurora did before,
  so the empty texels around a cut-out leave no trace. By default it is the
  plain average, as the Legacy Viewer's mips (`glGenerateMipmap`,
  `LLImageBase::generateMip`): the colour creators leave in the empty
  texels (often dark around grass and leaves) blends into the foliage seen
  from afar, which comes out darker. Under the library Midday sky, a grass
  bed seen from about 15 m was about 40 % brighter (linear) with the
  weighted average than next to the Legacy Viewer. The mips are made at
  decode and kept in the transcoded cache: delete the `.bc7` files of the
  textures compared (`<cache>/tex/<id>.bc7`) before each of the two launches.
- **`AURORA_SHADOW_FILTER=gaps|mapped|before`**: the sun shadow lookup as it
  was before its edges were cleaned (`cascade_shadow` in `common.wgsl`,
  `shadow_normal` in `object.wgsl`). `gaps`: 4 × 4 filter taps up to two
  texels apart, so flat steps between their ramps drew parallel lines along
  every shadow edge in the two finer cascades (now as many taps as texels
  covered, at most 7 × 7: the same penumbra). `mapped`: the lookup of an
  object moved off its surface along its normal-mapped normal, so the
  relief of the normal map showed as a row of dots along the shadow edges
  (now the face's own normal). `before`: both. To look at with
  `AURORA_DEMO_TOD=2`, `AURORA_DEMO_PBR_OVERRIDE=1` and
  `AURORA_DEBUG_SHADOWS=1`: the edge of the torus's shadow across the
  "Normal + specular" panel is a row of dots with `mapped`, and the shadow
  of our avatar on the plaza has several parallel edges with `gaps`.

- **`AURORA_SHADOW_BIAS=legacy|new|alt`**: the receiver bias of the sun
  shadow lookup (`cascade_shadow` in `common.wgsl`). `legacy`: a fixed 0.0005
  of the cascade's depth range, about 0.4 m in the finest cascade (its range
  is the square plus 400 m on each side), so a point only fell in shadow
  behind a caster more than 0.4 m nearer the sun and the limit where that
  distance crossed 0.4 m followed the stored depth texel by texel: sharp
  stairs on an avatar's back and on a skirt under the waist. `new` (the
  default): two texels of the cascade, in metres along the light. `alt`:
  one second in two each. In the demo at noon (`AURORA_DEMO_TOD` unset),
  camera behind our avatar: the stairs on the skirt under the waist.
- **`AURORA_DEBUG_SHADOWS=1`**: instead of the image, objects and terrain
  show the cascade each pixel reads — red, green then blue from the finest,
  mixed where two cascades are blended, black beyond the shadow distance —
  darkened where the pixel is in shadow (`debug_view` in `common.wgsl`).
  Also Preferences › Debug › Show shadow cascades.
  Used to compare two shadow paths whatever the lighting: capture the same
  frame with `AURORA_SHADOW_LEGACY=distance` and without, the dark areas
  must be the same in both, apart from their edges.
- **`AURORA_PROFILE=1`**: one `perf summary` line per second, plus one
  `perf settings` line when the settings change. The background fps cap
  ("Limit when unfocused") is ignored: a window without focus is still
  measured at full speed.
- **`AURORA_PROFILE_FRAMES=1`**: in addition, one `render profile` line and
  one `gpu profile` line per frame (renderer steps and GPU time per element
  of each frame).
- **`AURORA_OWN_MOTION_TRACE=1`**: on a grid with Aurora's movement
  (Preferences › Aurora › Movement, "Instant"), the `demo link trace` line
  of `AURORA_DEMO_LINK=…,trace` for each frame where our avatar moves, is
  led or still has a correction to fade (`t` counts from the launch); the
  end of the line gives the flight, the simulator's height, the lead's,
  the simulator's velocity and the drawn one, to read a take-off or a
  landing.
- **`AURORA_HOVER_CHECK=1`**: each answer about the object under the cursor
  (kept from a previous frame, or searched among the candidates of the
  picking index) is compared with the full search in the scene; the
  differences are logged under `hover check`, with, every ten seconds, the
  searches made and the answers reused. Each search also checks the index
  itself against the scene: the spheres it holds (`index entries … unlike
  their object`) and the classes of the objects tested whatever the ray
  (`classes unlike their object`); all these counters must stay at 0. Costs
  the full search each frame.
- **`AURORA_HOVER_LEGACY=1`**: the search for the object under the cursor
  becomes the former one again (`scene/hover.rs`): every object tested
  whatever the ray (avatar, rigged mesh) is a candidate at each stage, each
  triangle of an object that is hit is tested, and the HUD pick looks at
  all the objects of the scene. `ab` alternates the two ways, one second
  each in the same session: this is the comparison to make on a loaded
  machine. To read in the `hover summary` lines of the profile
  (`search=new|old`, see [Profiling](#profiling)).
- **`AURORA_CLICK_PICK=wait|check`**: by default, a click on the world
  (left click, Alt + click, right click, build tools) requests the depth
  under its pixel without waiting for it: the request leaves with the next
  frame, the answer comes back one to three frames later (one or two with
  an fps cap, three when the GPU is the limit), and the click is then
  handled with the cursor and the keys it had; what follows it (release,
  other buttons, mouse movement that moves the camera) waits behind it, in
  order (`app/click_pick.rs`).
  - `wait`: the depth is read right away by waiting for the GPU, as before,
    to compare (without the read of the drawn faces: a rigged mesh is then
    not picked, as before too): in a scene where the GPU is the limit, the
    main thread then waits 16 to 30 ms at each click (`between` in `max:`
    and `off_cpu:` of `perf summary`).
  - `check`: both reads are made and compared, one `click check` line per
    click: `depth identical` (it is the same texel of the same frame:
    `DIFFERENT` is a defect), `target identical` (the point and the object
    the click finds; `DIFFERENT` only if what is under the cursor changed
    during the frames of waiting), the time of the read that waits and the
    delay of the answer. Costs the wait of `wait` at each click.
  - With `AURORA_PROFILE=1` (or `AURORA_DEMO_CLICKS`), each click writes a
    `click pick:` line: `answered after 2 frames, 9.14 ms`, or `the read
    waited 25.56 ms` with `wait`. In demo as on the grid.

- **`AURORA_STILL_CHECK=1`**: one `still check` line per second, to find
  the cause of an image that shakes when nothing should move (camera locked
  on an object). It gives the number of frames where the camera, its focus
  point, its field of view and our avatar changed (and by how much at most,
  in mm), the objects whose faces were rebuilt, with the reason (`geometry`:
  other level of detail; `material`, `shape`: update from the simulator;
  `faces`, `rigged`, `skeleton`, `material-generation`), and the placed
  objects whose matrix changed (`worn` counts separately those worn by an
  avatar). At most six examples follow each line: identifier, type, level
  of detail, size, distance, movement announced by the simulator
  (`scene/still_check.rs`). In demo as on the grid.

**`AURORA_SCULPT_WATCH=<avatar UUID>`** samples only that avatar's attachment
tree every 500 ms and emits `sculpt watch:` lines when its state changes.
It records object/map identifiers, raw sculpt parameters, path/profile bytes,
expected/requested/bound geometry, CPU map dimensions and texture streaming
state. This distinguishes ordinary-prim classification, a null/missing map,
and geometry left bound while a sculpt waits. It changes no object, rendering
setting or asset request. No names, locations, texture pixels or service URLs
are added to these lines. In normal mode the human connects and positions the
camera; the diagnostic is disabled unless explicitly configured.

**`AURORA_DEMO_DESCRIBE=<frame>[,clipboard]`** opens the Dev Toolbox and
presses J1 "Debug to log" at that frame (`clipboard`: J2 "Debug to
clipboard", which also copies and logs `dev toolbox: J2 copied (N lines)`). The lines
follow at once in demo, or after 3 s when names of worn items were asked:
`describe:` (scope), `attachments of …` per avatar within 10 m, `attachment
'<name>' <root id> point …` per attachment (prims with no face drawn, faces
left out for a texture-entry alpha of 0 or without triangles), and for the
avatars within 3 m, or the selected linkset, one `prim …` line per prim with
every face: drawn (`[fi tex … rgba … legacy / glTF alpha mode, cutoff …
tex alpha … -> Pass]`) or `left out` with its reason. To check a change of
these lines, stand the demo avatar near the crowd (`AURORA_DEMO_POS`) and
grep the session log for `prim ` and `left out`.

## Profiling

With `AURORA_PROFILE=1`, each `perf summary` line covers one second. Two
threads share a frame: the **main thread** runs the simulation and the
interface and closes the frame as a packet; the **render thread** turns the
packet into GPU work while the main thread is already on the next frame.
With `AURORA_RENDER_THREAD=0`, everything that follows runs on the main
thread.

- `fps`, `frame` (average time of a frame, in ms), `p95`, `max` and `slow`:
  the number of frames beyond 1.5 × the median of that second (the hitches
  shown by the frame graph). The frame is that of the main thread: the time
  between two turns of its loop, waits included.
- `ft=`: the frames of the second by whole millisecond, the empty ones left
  out (`ft=5:120,6:61,14:1`: 120 frames of 5 to 6 ms, 61 of 6 to 7 and one
  of 14 to 15; `99` holds everything from 99 ms on). Unlike `p95` and
  `max`, these counts add up over a reading: the frames of 10 ms or more
  per minute, and the p99 and p99.9 of a whole session, come from their
  sum.
- **Main thread**: the average CPU time of each step of the frame
  (`events`, `social`, `sync`, `media`, `lists`, `stream`, `params`, `ui`,
  `render`…), whose sum gives `frame`. `render` is what rendering costs the
  main thread: the frame packet and the wait for the render thread to take
  it (all the encoding, swapchain wait included, with
  `AURORA_RENDER_THREAD=0`).
- `max:`: the steps whose longest time on a single frame reached 1 ms in
  that second, the longest first, including the renderer steps (`r_*`), the
  parts of the streaming and of the sync (`s_*`), of the frame parameters
  (`p_*`) and of the interface (`u_*`) and the `thread` fields, or `-` if
  there are none. A periodic slow frame appears there with the step that
  caused it, for example `max: media=6.10 render=2.31`, while its average
  stays tiny.
- `off_cpu:`: on the same principle, the steps of both threads that spent
  at least 1 ms **off the processor** in a frame (elapsed time minus the
  thread's CPU time, Windows and Linux), the longest first. A slow step
  listed there waited instead of working: a lock (the heap while the
  background tasks allocate, for example), or a core taken by another
  thread when the machine is loaded. The steps that hand work to the pool
  (`sync`, `lists`, `poses`, `r_finish`) wait there for their threads by
  construction; those that wait by nature (`render`, `limiter`,
  `r_acquire`, `r_present`) are not listed there.
- `thread=on|off`, then where the two threads meet:
  - `packet` (main thread, in `render`): the closing of the frame — the
    modified ranges of the scene tables in the frame's write journal, a
    copy of the CPU draw lists — and its hand-over;
  - `wait_render` (main thread, in `render`): the time spent waiting for
    the render thread to finish the previous frame (back-pressure: the
    render thread holds at most one packet, none waits behind). A frame
    that carries a screenshot is waited for entirely;
  - `rt_frame` (render thread): all its time on a frame, from the packet to
    the end of presentation, swapchain wait included (0 with
    `thread=off`);
  - `rt_idle` (render thread): the time spent waiting for the main thread's
    packet;
  - `pace` (main thread, in `limiter`): the frame pacing's wait for the
    GPU to finish the frame before, before the next frame reads its input
    (0 when the GPU is not the limit, or with `AURORA_FRAME_PACING=0`);
  - `input`: from the start of the frame, where it reads its input, to its
    presentation (the latency the pacing cuts; what the GPU still draws
    after `present` is not in it).

  A frame costs roughly the longer of the main thread's work (`frame` minus
  `wait_render` and `limiter`) and of `rt_frame`. `rt_idle` above 0 with
  `wait_render` close to 0: the main thread is the limit; the reverse: it
  is the render thread, or the GPU / vsync behind it when `r_acquire` or
  `r_present` hold most of `rt_frame`.
- **Render thread** (main thread with `thread=off`): the renderer steps
  (`r_*`: `tessellate` — the egui shapes cut into triangles —, `resources`
  — the replay of the write journal —, passes, egui, `finish`, `submit`,
  `present`, `acquire`, the wait for the swapchain image). With the render
  thread, they are those of the previous frame: the result of a frame comes
  back with the next hand-over.
- Main thread again:
  - the parts of the streaming work (`s_*`, in `results` and `stream`):
    `fetched` downloads handed to the streamers, `geometry` geometry built
    and placed in the arena, `decoded` other finished tasks, `tex_update`
    downloads and decodes started, `assets` meshes / animations / sounds /
    materials, `skin` skin bindings, `upload` textures sent to the GPU, of
    which `pages` new texture pages, `maintain` eviction and cache writes,
    `diag`;
  - the parts of the scene sync (in `sync`): `sync_list` removed and
    modified objects and the frame's list, `sync_plan` their placement,
    `sync_apply` moves and full syncs that fit in the frame's time budget,
    `sync_anchors` anchors of the skeletons and of the attachment points
    (writing, sending with the posed palettes, centre of the anchored
    objects), `sync_alpha` faces reclassified after the change of a
    texture's alpha class;
  - the parts of the frame parameters (`p_*`, in `params`): `screen` switch
    from loading to the world and teleport screen, `env` sky and water of
    the frame and their textures, `mirror` search for the nearest visible
    mirror, then the particles — `part_sources` systems set up or stopped
    by the object updates, `part_sim` simulation of each source,
    `part_list` instances of the frame sorted from the farthest to the
    nearest (`particles` gives their number) —, `lights` nearest local
    lights and `probes` reflection probes (search for the candidates by
    slices, slots, face to capture);
  - the parts of the interface (`u_*`, in `ui`): `bars` top and bottom
    bars, `tags` avatar name tags and floating texts, `overlays` beacon,
    HUD texts, build tools, debug and chat bubbles, `chat` conversations
    and contacts, `people`, `inventory` (inventory, appearance, AO),
    `menus` (context menu, confirmations, media, object windows, notecards,
    scripts, notifications), `places` (land, profiles, places), `maps`
    (minimap and world map), `perf`, `options` (environment and
    preferences), `rest` (teleport screen, bug report, closing; the whole
    screen outside the world) and `pass`, what egui adds around the
    windows. A closed window counts there what its code does anyway. Then
    `ui_shapes`, `ui_meshes` (one draw per mesh) and `ui_vertices` per
    frame, and `ui_vertices_max`, the largest number of vertices of a frame
    of the second: a peak of `r_tessellate` without a peak of vertices is
    not extra work;
  - the GPU time per element (`g_*`). The marks placed inside a pass do not
    wait for the end of the work of the draws that precede them: when the
    transparent faces are heavy, part of their lighting is counted in
    `g_post` (measured in demo: a share that changes from one second to the
    next). For them, read the sum `g_transparent` + `g_post`.
    `g_transparent` covers the sorted transparent faces, their interleaved
    glow, the debug and selection overlays, the particles and the end of
    the pass (in MSAA, the resolve of the image); water, the glow of the
    opaque faces and the impostors are not part of it. `g_post` goes up to
    the end of the pass of the worn HUDs: when they are displayed, it also
    counts the name tags of the world, drawn just before them, and the blur
    of the name plates (0.017 ms in demo with `AURORA_DEMO_HUDS=1` and
    `AURORA_DEMO_TAGS=voice`);
  - what the transparent faces draw: `blend` (faces), `blend_tris_k` (their
    triangles, in thousands), `blend_glow`, `blend_early` (share of the
    frames where their invisible fragments are discarded before lighting);
    what each lit fragment may have to walk: `lights` (local lights of the
    frame, 64 at most), `light_tiles` (share of the frames where it only
    walks those of its screen tile and of its depth) and `probes`
    (reflection probes);
  - the detail of `g_shadows_depth`: `g_cull` (GPU culling of all the
    views), `g_shadow0` to `g_shadow2` (the shadow cascades, from the
    nearest to the farthest) and `g_depth` (depth prepass and occlusion).
    This split is indicative: the GPU overlaps neighbouring draws, and the
    vertex work of a cascade spills over onto what follows it; to compare
    two paths, read their sum `g_shadows_depth`. Then, per cascade (suffix
    0 to 2), what the shadows draw: `sd` draws, `sd_cpu` those of the CPU
    lists (the terrain), `st_k` thousands of triangles, `sd_rig` and
    `st_rig_k` the share of the avatars and of the rigged meshes (the
    triangles only count the draws of the GPU lists). `shadow_path`: the
    shadow path of the second (`AURORA_SHADOW_LEGACY`);
  - `g_depth_msaa`: with MSAA, the prepass of the multisampled depth from
    which the scene pass starts (0 outside MSAA), also counted in
    `g_shadows_depth`;
  - `g_ssao`: the ambient occlusion alone, Legacy or GTAO (`AURORA_AO`),
    also counted in `g_effects`;
  - `g_contact`: the sun's contact shadows alone (`AURORA_CONTACT_SHADOWS`),
    also counted in `g_effects`;
  - `g_planar`, `g_probe`, `g_copy`, `g_ssr`: the rest of `g_effects`,
    after the occlusion and the contact shadows: the planar reflections
    (water, mirrors; the refracted water light is counted with them), the
    reflection probe face with its filtering, the copy of the scene for
    refraction and SSR (the cut-out pass and the submerged water particles
    are counted with it), and SSR;
  - `g_bloom`: the bloom levels alone (`AURORA_BLOOM`, 0 while off), also
    counted in `g_post`;
  - the draws and draw commands, the objects synced / rebuilt (`synced`,
    `rebuilt`: averages **per frame**; `rebuilt=0.2` at 175 fps is 35
    rebuilds per second, `AURORA_STILL_CHECK=1` tells which ones), the worn
    objects and rigged meshes placed by the GPU (`anchored`, which the sync
    no longer visits), the posed avatars, the bytes of records and palettes
    sent to the GPU, those of the culling tables (`cull_kb`: faces,
    objects, boxes per bone) and of the vertices deformed in place
    (`vertices_kb`: flexible prims), and those of the whole write journal
    of the frame (`journal_kb`), the texture memory (live textures, texture
    pages and their allocated memory), the geometry memory, and the objects
    whose full sync is waiting for a frame that has time (`sync_backlog`:
    new objects waiting for their first sync included);
  - `xmode=old|new` at the end of the line with `AURORA_SYNC_SLICE=alt`:
    the path of the sync during that second.

### Network

With `AURORA_PROFILE=1`, one `net summary` line per second during a grid
session (none at the login screen or in demo mode, which has no circuit).
Times are in milliseconds, distances in metres; counts are those of the
second, `-` means "not measured in this second". The Performance window shows
the input delay, the corrections, the loop pause and the event wait without
the variable.

- `ping`, `avg`: last round trip to the agent's simulator (measured every
  5 s) and the smoothed value the resend delay is computed from.
- `udp in= out= pkt/s`, then `in / out kb/s`: packets and throughput of all
  the circuits.
- `resent`, `given_up`: reliable packets we sent again, and those abandoned
  after their last resend. `dup`: packets the simulator resent although we
  had them (our acknowledgement was late or lost). `unacked`: reliable
  packets still waiting for their acknowledgement.
- `gaps`, `late`: sequence numbers the simulators skipped, and packets that
  arrived after a later one (a resend, or out of order). `gaps` minus `late`
  over a stretch of the log is what never arrived: a loss on the link, or a
  receive buffer that overflowed while the loop was busy.
- `loop_max`: the longest time the session loop spent on one wake-up (a
  packet, a tick, a command). During that time no circuit is read,
  acknowledged or answered: a value of tens of milliseconds is a pause of
  the whole network side.
- `events`, `left_max`, `wait_max`: events applied by the main thread, the
  most left in the channel at the end of a frame (its 8 ms box was full) and
  the longest time an applied event had waited since the network sent it.
  A position update that waited is drawn that much later.
- `agent_updates`, `input_delay_max`: AgentUpdates sent, and the longest
  wait between a change of the control flags (a key pressed or released)
  and the AgentUpdate that carried it. This is what the viewer adds to the
  ping before the simulator knows about a key.
- `corrections`, `largest`: reports of the simulator that put our avatar
  more than 0.5 m from where it was extrapolated (Firestorm's
  `AGENT_POSITION_SNAP`), and the largest distance of the second, moves of
  more than 10 m left out. A lagging simulator shows here as rubber-banding.
- `lead_max`: with Aurora's movement (Preferences › Aurora › Movement), the
  largest lead of our drawn avatar over the simulator's position in the
  second (about the speed × the round trip while walking or running), 0
  with Firestorm's. The lines `own motion learned: walk=3.20/6 … ramp … delay=+0.012/4`
  (at most one every 5 s, when something changed) give the speeds (m/s),
  climbing speeds, ramps (s) and the delay beyond the round trip (s) the
  prediction learned from the simulator's reports, each with the number of
  samples it comes from (0: still the starting value, `own_motion.rs`);
  `own motion: the simulator does not follow …` marks a lead given up (a
  wall, a push), `own motion: suspended …` three of them within 10 s.
- `http active= queued= done= failed= retries=`, then its throughput: asset
  requests in flight and waiting at the end of the second, finished, failed
  for good and sent again during it. The line `asset host <name> answers in
  HTTP/…` appears once per host: the protocol decides how many requests one
  connection carries.
- `event_queue idle= errors=`: polls of the simulators' event queues that
  ended without an event after being held 10 s or more (the normal case:
  about two a minute per simulator in a quiet place), and polls counted as
  errors (ended sooner, or refused). Fifteen errors in a row, each one
  waited for 3 s longer than the one before, close the queue; when it is the
  agent's region the session ends with "connection lost", as in Firestorm.
- `objects full= cache_hit= cache_miss=`: objects the simulators sent in full
  (ObjectUpdate, ObjectUpdateCompressed), and those they announced by their
  cache entry (ObjectUpdateCached) that the object cache held, or did not
  and were asked for in full. Second after second, the curve of an
  arrival: in a region already visited, whether it waits for the
  announcements or for the objects missing from the cache (with
  `left_max`: whether the main thread keeps up with them).

Three more lines are written without the variable, to read next to
`loop_max` and `gaps`:

- `udp socket buffers: receive … bytes (system default …), send …`: once per
  session, the sizes the system granted for the circuits' socket (800,000 and
  200,000 are asked for, as Firestorm does).
- `object cache for <region>: … entries, read in … ms`: the cache file of a
  region, read when its handshake arrives; the reply to the handshake leaves
  after it.
- `object cache written: … objects, … KiB in … ms`: a cache that changed,
  written every two minutes, when its simulator goes and at the end of the
  session. Neither the read nor the write is done by the session loop: their
  time must not show in `loop_max`.

### Every 10 seconds

With `AURORA_PROFILE=1`, six more lines every 10 s, in demo mode too, to
rank what a long session costs by area. Counts are those of the 10 s,
durations in ms; a duration's `n= p50= p95= max=` is read from buckets a
quarter of a power of two wide: the percentiles are within 19 % (never above
`max`, which is exact). The probes cost a relaxed atomic addition or two
where they count (a packet, a request, a decode job) and nothing without the
variable; the lines themselves are written by the main thread once in 10 s,
the objects walked once for `avatar summary` (well under a millisecond for
tens of thousands of objects), the threads listed by a thread of their own.

- `msg summary`: UDP messages, `in` then `out`: messages and kilobits per
  second, then the 15 heaviest types as `Name=count/kilobytes` (whole
  packets, headers and appended acknowledgements included; resends count
  again), `others=` for the rest. Then `objects`: objects received in full
  uncompressed (`full`, ObjectUpdate) or compressed (`compressed`),
  `terse` moves, `cached` announced by their cache entry, of which
  `cache_hit` held by the object cache and `cache_miss` not, `requested` in
  full (RequestMultipleObjects) and `killed`.
- `http summary`: `queued` asset requests waiting for a slot at the end,
  `queue_peak` the most during the 10 s; then one segment per kind of
  request seen — `texture`, `mesh`, `anim`, `sound`, `material`,
  `settings`, `landmark`, `asset` (bakes and others), `inventory` (folder
  fetches) and `cap` (the other LLSD capability calls; the event queue polls
  are not counted) — with `started`, `done`, `failed` (by class:
  `failed_none` no response, `failed_4xx`, `failed_5xx`, `failed_other`),
  `retries`, throughput, `in_flight` at the end and its `peak`, `latency`
  from the request's slot to its result (retries included) and, for the
  assets, `wait` in the queue before a slot. `hosts:` the 5 asset hosts
  that sent the most: `host=responses/kilobytes errors= ttfb_avg= ttfb_max=`
  (time to the response headers). A long `wait` with a short `latency`:
  the queue is the limit, not the servers.
- `asset summary`: `texture_decode` the decode jobs (JPEG 2000 to the
  staged chain), `bc7` the BC7 encoding of those that tried it,
  `decoding` the textures waiting for a decode or decoding and
  `upload_wait` those decoded waiting for the GPU (average over the frames
  and most), `j2k_cache` disk cache reads found or not and `bc7_cache_hit`
  textures read back as BC7 without a decode; `mesh_decode` a level of
  detail decoded into geometry, `mesh_parse` header and skin of the bytes
  downloaded or read, `mesh_cache` disk cache reads found or not,
  `meshes_fetching_max`; `jobs` the background jobs queued or running,
  `geom_pending_max`; `gpu_upload` the MB per second of textures and of
  geometry written to the GPU.
- `avatar summary`: in the last frame, the avatars of the scene (`total`,
  `max` over the 10 s), drawn in full (`full`, `full_in_view`), as impostors
  (`impostors`, `impostors_in_view`, `pictures/s` taken), as silhouettes
  over the complexity limit and hidden while loading; `complexity` sum,
  largest and average over the avatars it is known for; `attachments` worn
  (HUDs left out), those of the avatars drawn in full, `rigged` meshes and
  their triangles in thousands (`rigged_tris_k`, then `full=` for the
  avatars drawn in full) and `worn_tris_k_full` all the worn triangles of
  the avatars drawn in full, at the level of detail built; `animations`
  playing on the posed skeletons (and the most on one); `posed` skeletons
  per frame and `skeletons_rebuilt` per frame (a shape, an attachment or
  their age changed).
- `memory summary` (MB): the process's `working_set`, its `peak`, its
  `private` committed memory and its page faults per second; the GPU:
  `budget` the system grants the process, `usage` what the driver counts,
  `allocated` its live textures and buffers (VK_EXT_memory_budget, `-`
  without it), `other` what `allocated` holds beyond the texture pages and
  the geometry (render targets, shadow maps, tables); then the textures
  (`texture_mb` their levels, `pages`, `pages_mb` allocated),
  `texture_target_mb` and `pressure` of the texture budget, `geometry_mb`
  the geometry arena and `geometry_used_mb` what is used of it. On Linux
  `working_set` is the resident memory (`VmRSS`), `peak` its high-water
  mark (`VmHWM`) and `private` the anonymous memory, resident or swapped
  out (`RssAnon` + `VmSwap`).
- `cpu summary` (Windows and Linux), written by the `aurora-probe` thread:
  the process's CPU time as a share of one core and of all the cores,
  `machine=` how busy all the cores were (every process: another program
  loading the PC shows there), then each group of threads as a share of
  one core, the busiest first: `main`, `aurora-render`, `aurora-worker`
  (the frame's parallel loops), `aurora-jobs` (decodes, meshes, disk),
  `aurora-net` (the network runtime), `aurora-pages`, `aurora-trash`,
  audio, voice, `unnamed` (the driver's and the system's threads)…; a pool
  gives its threads and its busiest one (`aurora-jobs=320%(14t max=31%)`);
  the groups under 0.1 % of a core are only counted (`quiet_groups=`).
  `main` near 100 %: the main thread is the limit; a pool near its thread
  count × 100 %: that pool is; `machine` near 100 %: the PC is.

### Teleports and region crossings

With `AURORA_PROFILE=1`, a teleport or a region crossing writes one
`teleport timeline:` or `crossing timeline:` line when it ends: its
outcome and length, then every step as `+<ms since the start> <step>`, then
`link:` what the circuits lost meanwhile (`lost` = `gaps` − `late`,
`resent`, `given_up`, `dup`, `event_queue_errors`). Regions are named by
their grid coordinates (`1000,1003`).

- A teleport starts at the request sent (TeleportLocationRequest,
  TeleportLandmarkRequest, TeleportLureRequest) or at a TeleportStart the
  viewer did not ask for (`begun by the simulator`); its steps:
  TeleportStart, each TeleportProgress with its text, TeleportFinish, the
  new region's circuit (`UseCircuitCode sent`, or `already open for … s` when
  it was a neighbour, and whether it was acknowledged), `circuit … up`
  (UseCircuitCode acknowledged), the seed capability asked and its reply
  (number of capabilities and time, or the error), `event queue … started`
  and its first event, EstablishAgentCommunication, CompleteAgentMovement
  sent, AgentMovementComplete received, the viewer applying the arrival
  (seated or not, objects worn), RegionHandshake and its reply, the first
  object update of the region, the first update of our avatar after the
  arrival (and how far it was from where it was drawn), the previous region
  kept as a neighbour or its circuit closed, the appearance or outfit
  version asked, corrections of our avatar's position. It ends `arrived
  (teleport screen hidden)` (`…, old screen)` with `AURORA_TP_SCREEN`),
  `failed (TeleportFailed: <reason>)`, `failed (no
  arrival 90 s after the teleport screen was shown)`, `arrived` 20 s after
  AgentMovementComplete without a screen, `timed out` after 90 s, or
  `interrupted by the next one`. A teleport inside the region ends 2 s after
  its TeleportLocal.
- A crossing starts at the CrossedRegion event; same steps from the
  circuit on, then `arrived` 5 s after AgentMovementComplete (the
  corrections and the first update of our avatar in those 5 s show how far
  it snapped; between CrossedRegion and AgentMovementComplete no
  AgentUpdate leaves: the avatar does not answer the keys), or `timed out`
  after 30 s.
- `region link:` lines, as they happen: EnableSimulator,
  EstablishAgentCommunication, UseCircuitCode sent, circuit up (or not
  acknowledged), event queue started or given up, DisableSimulator, circuit
  closed (with how long it was open). Inside a timeline they are steps too.
- The demo's remote teleport (`AURORA_DEMO_TP=remote`) writes a timeline
  with its simulated steps only (no simulator: no messages).

### Object under the cursor

With `AURORA_PROFILE=1`, one `hover summary` line per second as long as the
cursor is over the world (none when it is over the interface, in mouselook
or when Alt, Ctrl or Shift is held): what the search for the object under
the cursor did (`scene/hover.rs`), apart from the `hover` step of
`perf summary`, which also counts the HUDs and the setting of the cursor.

- `search=new|old`: the way of searching of that second
  (`AURORA_HOVER_LEGACY`);
- `hud_ms_per_frame`: the time of the HUD pick, made each frame before
  searching in the world;
- `asked`: the frames that requested the object, `searches`: those where it
  had to be searched (the others kept the answer), `ms_per_frame`: the
  search time spread over each frame, `longest_ms`: the longest search;
- `why:` why the kept answer no longer held (a search can count in
  several): `none` no answer, `ray` the cursor or the camera moved, `depth`
  the depth read under the cursor changed, `index` an object moved or
  changed near the ray, `refresh` the answer was 100 ms old;
- `index:` what disturbed the index first: `bounded` an object with its
  bounds, `always` an object tested whatever the ray (avatar, rigged mesh),
  `changed` an object updated without moving, `reset` everything was
  removed;
- `per search:` per search, the candidates of each stage — `ignore` (IGNORE
  objects that stop the ray), `see_through` (transparent faces with an
  action), `point` (object at the point found) —, the time of the pass over
  the index (`index_ms`, classes to establish included), of the search for
  the point (`point_ms`) and for the object at that point (`object_ms`),
  then the exact tests: `tested` objects whose triangles were requested,
  `boxes` those whose box the ray hits, `triangles` triangles tested one by
  one, `hierarchies` faces that went through their hierarchy instead;
- `ignored`: the searches that met an IGNORE object (they take all the
  candidates again), `classes_set`: the classes established in the second,
  `repaired`: the classes found wrong by the rotating verification since
  launch (must stay at 0).

### Texture loading

Without any variable, the log receives a `texture load:` line at the end of
each texture load (`scene/texture_load.rs`): from the first file read,
downloaded or decoded until nothing is left to read, download, decode or
send to the GPU for 2 s. Small loads (fewer than 200 decodes and less than
5 s) do not get one. A loaded region never stops completely (a few
downloads always remain in progress): as long as a load lasts, a
`texture load (running):` line gives its progress every 10 s, and the
arrival in another region (login, teleport) closes the one in progress
(`cut by an arrival`) so that the next one only counts the new region. Each
line gives:

- the duration of the load, the "time until everything is loaded" to
  compare from one version to another and with Firestorm at the same place,
  and the textures loaded out of those known;
- the JPEG2000 decodes, including those of a texture already displayed at a
  coarser level and those whose texture goes to BC7, and their number per
  reduction level (`discard 0..5`, 0 = full resolution); the megapixels
  decoded;
- the CPU time of the decoding tasks, all threads added up, and its
  breakdown: `codec` (OpenJPEG), `convert` (32-bit planes to 8-bit pixels),
  `rgba` (conversion to RGBA and alpha class), `mips`, `bc7` (BC7 encoding
  and quality check), `staging` (picking mask and copy to staging memory);
- the bytes downloaded and the number of responses, the successful and
  missed reads of the disk cache (JPEG2000 data), and the textures read
  back from the transcoded cache (`read as BC7 without a decode`): sent to
  the GPU without decoding or encoding;
- `first pixels of … downloaded textures … s after their request on
  average, … s weighted by their size on screen`: for the textures that
  had to be downloaded, the time between their first request and their
  first pixels on screen. The second figure weights each texture by the
  area it covers: how long a pixel of a new texture stays without its
  picture, which is what an arrival looks like. `… asked before that size
  was known` counts the first requests sent for a texture not seen on
  screen yet (out of view, or just arrived): they leave at the lowest
  rank and for the smallest range.

While meshes download, a `mesh load:` line (`scene/meshes.rs`) gives every
10 s the requests sent and the megabytes of mesh assets downloaded since
the start of the session, and the meshes loaded, downloading and failed:
beside the bytes of the `texture load:` line, what an arrival costs in
meshes.

The cost of decoding alone is measured outside the viewer on a folder of
cached textures (read-only), per size class, on one thread then on as many
threads as the background tasks:

```powershell
cargo run --release -p aurora-assets --example j2k_bench -- "$env:LOCALAPPDATA\Aurora\AuroraViewer\cache\tex"
```

Options: `--sample <n>` (2000 textures by default), `--threads <n>`,
`--levels 0,1,2,3`.

What BC7 compression gives on the same folder (share of the textures that
pass the quality check, video memory with and without it, cost of encoding
against decoding), opaque textures and textures with alpha apart:

```powershell
cargo run --release -p aurora-render --example bc7_check -- "$env:LOCALAPPDATA\Aurora\AuroraViewer\cache\tex"
```

Options: `--sample <n>` (1000 by default), `--discard <n>` (decoded level,
0 by default).

#### Quiet bridge discovery (1.17)

The bridge no longer announces its HTTP-in URL every two seconds until acknowledged. Aurora requests discovery on a negative channel through ScriptDialogReply (agent ObjectID, zero button index, exact worn root in the label), at most every five seconds while connecting/recovering. The script accepts only its owner's exact request and applies its own five-second throttle. The offline bridge simulator answers these commands; it no longer supplies unsolicited periodic hellos. Tests cover request loss, delayed startup, exact object/region checks, rediscovery after a reset endpoint and stable Movelock selection. Actual-LSL discovery evaluation also checks silence without probes, owner/message/channel filtering, delayed permissions/URL order and a single replacement-endpoint notification. These checks are offline and do not validate simulator delivery.

Human follow-up, after Aurora has installed script 1.17: normal startup/region changes must reconnect, and leaving the attachment worn on another viewer must not produce the old repeated discovery messages or detach it. A script 1.15/1.16 still worn from before this update retains its old behavior until Aurora upgrades it. The 1.16 remote-anchor passage was reported unsuccessful by the human (0.19 m progress, 1.21 m remaining, 4.6 m/s highest sampled speed); 1.17 changes discovery only and does not claim to fix physical wall penetration.

The first 1.17 viewer incorrectly sent ChatFromViewer on that negative channel. The simulator rejects it; the revised offline receiver now rejects it too. The corrected viewer uses ScriptDialogReply as Firestorm does, with the existing compatible 1.17 LSL source. For diagnostics, combine AURORA_DEMO_MOVEMENT=surface with AURORA_DEMO_MOVELOCK=error|timeout|endpoint: AO Movement must show the synthetic compilation failure or actual startup stage. HTTP cause propagation and recovery are covered separately by unit tests. These remain synthetic errors. Real HTTP errors expose only a safe timeout/network/status/format/sequence cause, never raw request errors, private URLs or response bodies. A passage refused before sending is labelled as such; its stored result is separate from current bridge availability.

The maintainer confirmed successful wall passage with the corrected discovery transport and script 1.17 on 2026-10-16. The follow-up viewer extends obstacle selection from 1.5 m to 3 m measured from the avatar centre. The existing geometry regression checks approaches at 1.6, 2.5 and 3 m, an identical far-side exit, rejection beyond 3 m and the retained occupied/phantom rejection. Human follow-up is triggering from farther away; the physical motor and maximum 8 m travel are unchanged.

Bridge 1.18 tightens only the passage anchor response from 50 ms to 45 ms at the maintainer's request for a small traction increase. Actual-LSL evaluation checks the passage response against the documented 2/45 s lower bound, the unchanged normal Movelock response/vehicle parameters, fixed goals/replays, completion/deadlines, cleanup and ownership. This does not simulate Havok or establish an exact force gain. The existing 3 m obstacle reach, 8 m travel and 1.2 s server lifetime stay unchanged. Human follow-up is reliability on the same obstacle with the upgraded 1.18 script.

The farther-goal follow-up adds 0.45 m to passage travel: the 1 m cube centred at x=129 now yields x=130.5, one metre beyond its far face at x=129.5, rather than x=130.05. The existing geometry regression asserts this destination from near and 1.6/2.5/3 m approach positions, rejection of an occupied destination and obstacles beyond reach, and phantom handling. Detection (3 m), maximum travel (8 m) and the compatible 1.18 motor are unchanged. Human follow-up is checking whether the farther anchor improves consistent clearance on the same wall.

Water-entry regressions now cover a fast fall sampled first near the waterline and then several metres below it: automatic ascent persists until the configured flotation height, produces the AO SwimmingUp signal and hands back to surface buoyancy without native flight. Test all flotation offsets (-0.50/0/+0.50 m), descent held on entry or pressed during ascent, persistent voluntary depth after release, unreachable-target expiry, disabled/sitting/suspended/missing-bridge/shallow/missing-water cleanup, and normal Movelock precedence. The existing tap-to-dive and tap/hold ascent cases remain separate. Offline bridge motor checks cover the unchanged bounded mode 8 goals and watchdog; they do not simulate Havok. Human follow-up: fall into deep water with swimming enabled and no vertical input, verify return to flotation without exiting into flight, then verify a deliberate dive still stays underwater.

Surface-return follow-up: regressions compare automatic entry with an actual underwater up-tap goal; sample a fall through the exact flotation height before a further 20 m descent and a slow ascent lasting over 30 seconds. The mode must remain Surface return until flotation is reached at low vertical speed. Fresh up/down interruptions, release without replay, a later restart, held-up flight, entry after a shore exit, blocked-target expiry and lifecycle cancellation remain covered. The source budget is renewed by observed movement, not elapsed time alone. Human follow-up is the same fall that previously stopped early, plus up/down interruption before arrival. No LSL version change is required.

Gentler surface-return follow-up (bridge 1.19): actual-LSL evaluation covers command caps at selected speeds 0.5/1/2/5/8 m/s, the 1.5 m/s-per-second vertical command ramp, smaller goals near flotation, bounded goals that still converge, and release/mode-switch/Movelock resets. Its deterministic position tracker is not Havok and cannot establish real acceleration or smoothness. Existing water/gesture/watchdog/ownership, accepted run, remote-anchor and quiet discovery checks remain covered. Human follow-up is the same fall and short underwater ascent tap: the lift should start gently and slow before reaching the configured flotation height; up/down interruption remains available. The viewer must install the compatible 1.19 script into the existing bridge.


**`AURORA_DEMO_BRIDGE_QUERY=radar|scripts|object|error`** — authenticated read-only bridge services (1.21). Set `AURORA_DEMO=1` and use the usual automated-window title/border. The scenario provisions the real offline factory; all BridgeQuery commands receive synthetic replies without using a network endpoint. `radar` adds Sky Visitor with an unknown coarse altitude (1020 m), opens People / Nearby and returns 2048 m through the bridge service; its displayed distance must become approximately 2 km. Known native/coarse heights remain authoritative and are not queried. The demo opens the real radar options to inspect the simple correction toggle without diagnostic counters. `scripts` asks for a demo object at frame 700; local chat must show 12/17 running scripts, 1088 KiB allocated memory, 1.250 ms CPU and 0.200 ms pathfinding (localized decimal separators). `object` dispatches the real Object information context action at frame 700 and reports the basic script summary followed by all extended object fields (three prims, land impact two, nineteen inventory items, four-metre distance, identity links, UTC timestamps and attachment status), without changing a preference. `error` produces a safe missing/unavailable-object report rather than a raw HTTP body or private address. Capture at frame 1050 or later, after the factory save/reattach delay. These fixtures cover UI and dispatch, not the real LSL runtime.

Read-only bridge unit coverage includes batch/rate limits and fair coverage of a 320-avatar crowd, partial/empty results, strict requested-UUID/numeric/name decoding, known-position priority, fourteen-second expiry, region/session/teleport cleanup, disabled correction, inspection priority/provisioning/timeouts and stale replies. A failed current query triggers only quiet throttled discovery, preserving the confirmed lock and in-flight motor command; obsolete replies cannot initiate recovery. The actual shipped LSL query/auth control flow is also evaluated with deterministic API stubs for bounded requests, token rejection, unavailable targets, unusual names and metric units, asserting no writes to motor globals, sequencing or watchdog. Existing run/water/passage/discovery regression evaluators still apply; this is not a simulator/Havok test.

Human grid validation: let the existing bridge upgrade to 1.20 in `#Aurora/Aurora Bridge`. In People / Nearby, check an unloaded avatar above the coarse altitude limit, toggle the radar's bridge-correction option, then change region to verify stale corrections disappear. An inaccessible avatar must keep an unknown distance. Right-click a scripted object (including a child prim) / Object / Script information and compare its root's running/total counts, allocated memory and CPU with Legacy Viewer; repeat with a zero-script object, an attachment and an object that disappears. Results appear locally and must not send ordinary/public chat. Confirm Movelock, flight/swimming/run and quiet idle bridge behavior remain unchanged during queries. Real grid validation remains the human's responsibility; do not automate login or inspect their online session/logs.


Read-only bridge validation (2026-10-16): 2,843 workspace tests pass, 24 ignored, with the known unrelated GPU transparency test skipped. Script parsing, formatting, Clippy and LSL lint pass; the default dev viewer embeds the full 1.20 source byte-for-byte. Inspected `.scratch/bridge-query-radar-fr.png` (Sky Visitor: 2015.2 m from the demo avatar at about 33 m, using the bridge's 2048 m altitude), `bridge-query-scripts-fr.png`, `bridge-query-scripts-en.png` and `bridge-query-error-fr.png`. Reports are readable with localized metrics and safe failure text; their dedicated offline logs contain no panic/error. These checks do not validate real simulator queries or Havok. Human grid follow-up above remains pending.


Cleanup regressions (2026-10-16): verify known native/coarse heights never issue a bridge query, cached/late replies cannot override a newly known 250 m height, and an unloaded avatar with missing coarse height becomes a 2480 m range from a 20 m observer after a 2500 m bridge reply. Parent-chain coverage retains missing direct/indirect seat ancestors, a rotated complete chain and cyclic parents. Nil IDs must not invalidate a batch containing a valid missing-height target. The speculative stale/native verification scenarios and status-counter UI were removed at the maintainer's request after they clarified that the actual target altitude was 250 m, not 2500 m. The approximately 235 m real-region case and script information were confirmed by the maintainer; real-grid lookup above the coarse altitude limit remains unconfirmed. LSL and physical motors are unchanged. Cleanup validation: 2,848 workspace tests pass, zero failures, 24 ignored, with the known unrelated GPU transparency test skipped. Scripts, formatting, final Clippy and the default dev build pass. Inspected .scratch/bridge-query-radar-fr.png and bridge-query-radar-en.png: both menus retain the simple default-on toggle without counters, and the missing-height target shows a distance of about 2 km. Dedicated radar demo logs have no panic/error. The unchanged complete LSL 1.20 source is embedded byte-for-byte; source hashes confirm the read-only protocol, physical motors and swimming routing are unchanged. The normal test viewer was relaunched as Test aurora-bridge-movelock radar cleanup 1.20, without automatic login.

**`AURORA_DEMO_AO=firestorm|empty`** — AO startup reconstruction. `firestorm` seeds a synthetic Legacy Viewer catalogue and an empty shared Aurora parent; the real bootstrap and inventory-mutation callbacks must create #Aurora/Aurora AO, preserve sets/links/options and the four actual legacy swim mappings and select the remapped set without changing any #Firestorm folder or item. Floating, Swimming Forward, Swimming Up and Swimming Down (including their Swim.H/N/U/D aliases) must be fetched and copied when present; The approved completion copies available donor configurations into Aurora-only slots. A missing donor for a required slot must keep swimming unavailable; copying the set must not enable the swimming preference. `empty` opens an AO without either source and must create no AO data or upload prompt. `swim` remains the visual check that the old import-pack button is gone in English/French. All scenarios are offline and never log into a grid.

AO startup regressions cover existing (including empty) Aurora-folder precedence, complete Firestorm set/group/track/cycle reconstruction, original-link preservation, saved selection remapping, missing source/legacy swim items, incomplete fetches and missing original items, interrupted staging without duplicate replay, and shared-parent creation coordination. A bridge factory regression provisions and updates its script without an AO or enabled Movelock. Human grid validation remains required for first-login AIS behavior, a second login with existing Aurora data, absent source data, and animation visibility from another viewer.

AO startup validation (2026-10-16): 2,854 workspace tests pass, zero failures, 24 ignored, with the known unrelated GPU transparency test skipped. Scripts, formatting, final Clippy and default dev build succeed. Inspected .scratch/ao-startup-swim-fr.png and ao-startup-swim-en.png (ordinary swimming inventory editor, no pack-import button), ao-startup-firestorm-fr.png (reconstructed catalogue, saved set and working cycle) and ao-startup-empty-fr.png (no set and no upload prompt). Their dedicated offline logs have no panic/error. Source hashes confirm the bridge factory, full LSL 1.20, physical motors and AO animation runtime are unchanged; the executable embeds that full script and no longer contains the removed pack action/description. Normal viewer relaunched as Test aurora-bridge-movelock AO startup 1.20, without automatic login. Real-grid first-login reconstruction remains for human validation.

**`AURORA_DEMO_AO=firestorm-wait`** — cached inventory with delayed write/readback services. Seeds a synthetic MonSet with Standing:CT15.00:CY:RN under #Firestorm/#AO and no native AO. Before frame 720, the ordinary AO window must show loading and no new AO folder. At frame 720 both synthetic capabilities become available through the same writable gate as production; the real bootstrap must reconstruct exactly one #Aurora/Aurora AO/MonSet, preserve the standing options, the four actual swim mappings and the original source. Capture at 620 (waiting) and 1000 (saved); offline only.

**`AURORA_DEMO_AO=migrate|migrate-done|migrate-swim`** — explicit Legacy Viewer migration beside an already existing native AO. Both modes seed two synthetic legacy sets and one native default, so automatic fallback must do nothing. `migrate` opens the real picker at frame 360, initially showing the duplicate-name guard for Aurora AO. Choose Firestorm Walk Set or change the destination name, then migrate; cancel must leave inventory and the native selection unchanged. `migrate-done` chooses Firestorm Walk Set and invokes the real action at frame 720; capture at 1000 to check the new selection and saved status. The native default is retained, only one set is added, the legacy source stays unchanged and the four actual swim mappings retain their original animation targets, order and cycles. Aurora-only slots reuse the approved existing donor animations with new native links; missing donors remain empty. `migrate-swim` performs the same migration and reveals Underwater floating at frame 900; capture at 1100 to check the copied animation and the completed gate. These fixtures are offline only.

**`AURORA_DEMO_AO=migrate-alchemy|migrate-alchemy-done`** — the same fixture plus one synthetic Alchemy set, Alchemy Dance Set, in an `Animation Overrides` folder at the inventory root. `migrate-alchemy` opens the picker with Alchemy chosen and that set selected; switching back to the first choice lists the two legacy sets again and drops the Alchemy choice. `migrate-alchemy-done` migrates Alchemy Dance Set at frame 720; capture at 1000. The native default is retained, the Alchemy folder stays unchanged and the four swim slots are completed like a Legacy Viewer set.

AO interface integration validation (2026-10-16): scripts and formatting pass; Clippy is clean with warnings denied; 2,882 workspace tests pass, zero failures and 24 ignored, with the existing unrelated GPU transparency test excluded. The native row test performs pointer press/drag/release and reads back the acknowledged inventory order. Additional coverage checks guided cancellation, remapped/scoped editor selection, member/track ordering, group dissociation, original preservation, independent cycles and programmatic swimming-state reveal after the native smooth-scroll animation. The default dev viewer builds. Inspected owned `.scratch/ao-interface-interface-{fr,en}.png`, `interface-group-{fr,en}.png`, `interface-combine-{fr,en}.png`, `interface-swim-{fr,en}.png`, `interface-track-fr.png` and `movement-{fr,en}.png` (all with the `ao-interface-` prefix): controls and text fit the unchanged Aurora theme, swimming selection is visible in the rail and its remove action shares the picker row, and the movement layout retains its independent switches and diagnostics. Scoped offline logs contain no error/panic. The theme, fonts, shared widgets, complete LSL source and physical controller sources match their pre-integration hashes. Real-grid confirmation of grouping, dissociation and persisted ordering remains for the human.

Human acceptance (2026-10-16): the maintainer approved the integrated AO interface and requested merging the bridge/AO branch. This approval does not add automated or observed grid coverage beyond the cases documented above. The movement helpers remain explicitly EXPERIMENTAL; simulator-specific animation transitions, manual migration and persisted group editing remain follow-up checks.

Merge validation after main integration (2026-10-16, PR #493): PowerShell parsing, formatting and workspace/all-target Clippy pass; the full unfiltered workspace test run reports 3,210 passed, zero failures and 24 ignored. The previously excluded transparent-mesh GPU test now passes on this combined source. Bridge reception preserves the original datagram timestamp, with a buffered-motion regression; the toolbar catalogue lists each new feature once and retains the legacy move_lock alias. Shortcut coverage preserves the newer Ctrl+Space library binding when jump is rebound to Space. Object-menu regression expectations now recognize bridge script information as available. Public protocol capability names have two narrow, explained Gitleaks annotations. LSL lint reports zero errors/warnings; deterministic actual-source evaluations retain 15 quiet-discovery cases, 81 run cases, 50 passage cases and water/surface-return ownership and bounds checks; these are not Havok simulations.

Construction inspector proposal (2026-10-16): use the General, Transform,
Materials and Create demos to inspect title permission states, numeric step
buttons, inline material options, shared rez-under-land-group, and the vertical
coordinate column. On the grid, the human checks both picker resource types,
all three sources, collapse/reopen, UUID acceptance/rejection and cancellation.
Link two independent objects and wait for simulator confirmation: one root,
child-colored outlines, one whole-object selection and consistent bounds.
Compare linked-prim navigation with script-side link numbers, especially for
pre-existing linksets; UDP ObjectUpdate carries ParentID but no link number.


### Interactive water


Anatomical contacts: append `:parts-feet`, `:parts-feet-run`, `:parts-hands` or `:parts-head` to
a water quality, with `AURORA_DEMO_WATER_ACTIONS` unset. Each isolates one
native avatar: stepping feet, arms stirring while the root stays still, or
head immersion/emergence. Shore foam, caustics, the moving hull and the fixture's
system skirt are omitted to expose the local contact; macro waves stay enabled.
There must be separate foot/hand traces,
no automatic torso-wide V, no trails from dry parts, and only sparse head air.
Foot slaps occur once per lifted foot's descent into water: a short low sheet,
scattered small droplets and a restrained ring. Running produces stronger,
quicker contacts; dragging a submerged foot and waves passing an idle foot
must not manufacture new slaps. An ankle-deep lifted heel can slap even when
the whole sole does not clear the surface. The fixture closes restored demo
floaters and reasserts its camera after loading, keeping the contacts visible.
Continuous furrows should be gentle at low
speed and small limbs must not displace a torso-sized volume of water.
Combine the ordinary `swim`, `swim-turn` and `wade-run` action fixtures for the
supplied clips and translating avatar. Recheck each effect/quality, actual
underwater transmission, dive/cannonball entry and dry/hidden body regions.
The fitting uses animated collision volumes and visible skin influences;
custom meshes and per-texel alpha cutouts remain approximate, so the human
checks their actual outfit and animations on the grid. No supplied animation
is imported or modified. Tests cover material-point motion, independent parts,
fast crossings, head entry/exit, visibility ancestry, teleports, 30/60/144 fps,
particle independence and sparse per-head air. The GPU atlas regression also
checks a small vertical strike and CPU contact height against the actual
rendered Gerstner spectrum at all qualities and several depths.

| Variable | Purpose |
|---|---|
| `AURORA_DEMO_WATER=medium` | Offline coast, moving hull, pillars, wading and submerged avatars |
| `AURORA_DEMO_WATER=off:exclusion` | Invisible water exclusion above/below the surface; use `medium` for enhanced water |
| `AURORA_DEMO_WATER_ACTIONS=dive` | Local native water animation and jump/entry preview |
| `AURORA_DEMO_WATER_ANIMS=<folder>` | Read-only folder containing the twelve native water clips |
| `AURORA_DEMO_WATER_SHADERS=1` | Load water shader sources for offline material iteration |

Water exclusion: compare `off:exclusion-control` with `off:exclusion`, then
`medium:exclusion-control` with `medium:exclusion`. The left red floor must
remain visible, with water removed only through the invisible face above it.
The right green floor has an exclusion below water: the nearer water remains.
No grey texture, scene-depth occlusion, shadow or reflection should appear.
Repeat with `AURORA_CPU_CULL=1` and with MSAA; Ctrl+Alt+T reveals the hidden
faces. Actual edited faces and persistence across reconnect need human grid
validation, following the same checkbox/confirmation flow as the Legacy Viewer.

The enhanced-water master is disabled by default (2026-10-19), including missing/partial saved preferences and Aurora section reset. Individual effects stay selected for an explicit opt-in; saved on/off choices survive reload and graphics-preset changes. The explicit "All Aurora" bulk choice still enables water effects. Only the global activation carries the shared experimental warning triangle and its localized tooltip in Preferences › Aurora › Water. Compare disabled and enabled FR/EN captures: individual controls have no warning marks, disabled effect controls stay readable and the global warning must not overlap text or its switch.

Experimental water defaults validation (2026-10-19): final `scripts/check.ps1` green, 3,576 passed, zero failures, 26 ignored; scripts/formatting/Clippy clean. Regressions cover fresh/missing/partial defaults, disabled render flags, saved choices, all graphics presets, Aurora section reset and explicit bulk opt-in. Built the default dev viewer and inspected `.scratch/ao-water-spacing-final-fr.png`, `ao-water-spacing-final-en.png` (1024x768), `water-experimental-off-fr.png` and `water-experimental-on-en.png`, compared with `ao-optional-water-controls-fr.png` and `water-experimental-before-fr.png`. The two gesture gaps are wider and scrolling remains available. Water preferences show exactly one warning on global activation, with ordinary individual controls and readable disabled choices. Both water modes are explicit offline fixtures; they do not verify a real user's saved preference. Last-five disabled-water preferences before/after medians: 60 FPS, UI 0.83/0.80 ms and GPU 1.95/1.59 ms; baseline compilation and desktop load vary, so no isolated performance gain is claimed. Preferences captures have zero warnings/errors/panics; AO captures have no errors/panics and only the baseline synthetic missing-outfit warning. Human acceptance remains required before PR.

Modes `off`, `low`, `medium`, `high`, `ultra` explicitly select the demo's initial master/quality, overriding the normal disabled default; `medium:underwater` (or another quality with `:underwater`) starts the camera below the surface. The `cycle` mode (also `cycle:window` for the underwater camera) switches the master and each individual effect, cycling all four qualities every 60 frames; its log records each transition. Camera suffixes `:shore` and `:close` inspect grazing reflections/shore breakers and detailed hull/foot contacts; `:foam` inspects close shoreline coverage and reflected highlights, and `:avatar` inspects wading droplets (`:avatar:run` raises horizontal speed to exercise running) (also test with `AURORA_DEMO_TOD=3` for sunset lighting); `:underwater` inspects refracted caustics and bubble lighting. `:surface` follows a horizontal swimmer; `:window` looks up through the water at a colored above-water crossbar, and `:window-grazing` inspects internal reflection at shallow angles. `:alpha` (#1678) adds three semi-transparent boxes, green under the water, yellow across its surface and magenta above it: from above (`off:shore:alpha`, `medium:shore:alpha`), the green one and the lower half of the yellow one must be refracted and fogged by the water like the ground around them, never drawn crisp over the surface, while the upper half and the magenta one look as before; from below (`:underwater:alpha`) nothing changes. Only demo mode reads it. The inspection view stays fixed across frames unless free camera is active; action scenarios retain their own framing. Preferences remain editable. Combine with `AURORA_DEMO_OPTIONS=graphics/3:cogs` for Preferences › Graphics › Reflections and water (Aurora water); `AURORA_AO=gtao:2` for the GTAO integration; `AURORA_PROFILE=1` for CPU/GPU timings. Capture several settled frames: shoreline and pillar foam should persist, the moving hull should leave an expanding wake, immersed avatar motion should emit bubbles, and droplets should fall back into the water. From above, submerged particles enter the refracted scene; below, they are seen directly through water fog. Switch each effect independently, change quality, and disable/enable the master switch: saved choices remain intact, physics and motion do not change. Also test MSAA 4×, SMAA, TAA, a night sky, a region crossing, and a teleport (no stale trails). Grid validation requires the human: boats, scripted objects, attachments, animated/custom avatars and region water at different heights. These effects target native region water, not arbitrary mesh pools.

With `AURORA_DEMO_WATER_SHADERS=1`, an offline viewer loads common/spectrum, scene object/environment, water simulation, swept-wake and caustics WGSL from its own source checkout when pipelines are created. Restart the demo after editing shaders; no build is needed for each visual comparison. This switch is ignored without `AURORA_DEMO`; normal viewers and final acceptance runs use baked shaders. Missing source files fall back to the baked version.

Water contact checks: foam must have continuous torn edges without a regular dotted pore pattern, including close views and low quality. Static terrain must not acquire object-contact foam. Walking/running and surface swimming must leave a visible short wake and modest bow spray even with a pose without foot downstrokes. Individual shallow downstrokes add droplets; deeply submerged bodies/limbs create no spray through the surface. Only the head releases air while immersed and moving/entering, at most one packet every 1.25 seconds: two small bubbles at low quality, three at medium, four at high/ultra. The water underside must show the exterior crossbar and sky through a wave-distorted window, with submerged-scene reflection toward grazing angles. Moving a target farther along the same ray after its exit into air must not add underwater fog. Idle avatars emit none. Ripples can remain active with spray and bubbles disabled; switching particle effects must not change displacement. The GPU packs particle emitters separately from particle-free events and honours the shared particle budget.


Water performance A/B (2026-10-19): compare `AURORA_DEMO_WATER=off:shore`
with `medium:shore`, then `off:underwater` with `medium:underwater`, in the
same optimized Dev executable. Both modes use quality 1 and identical
individual-effect choices; only the master changes, with base water retained.
Set `AURORA_DEMO=1`, `AURORA_GRAPHICS_PRESET=high`, `AURORA_AO=gtao:2`,
`AURORA_AA=6`, `AURORA_WINDOW_SIZE=1920,1080`, `AURORA_NOVSYNC=1`,
`AURORA_PRESENT_MODE=immediate`, `AURORA_FPS_LIMIT=500` and `AURORA_PROFILE=1`.
Clear other demo switches. Exit through a unique `AURORA_CAPTURE` with
`AURORA_CAPTURE_EXIT=1`, `AURORA_CAPTURE_FRAMES=12000`; use the automated
window title/border. Run order: shore A/B, underwater B/A; shore B/A,
underwater A/B; shore A/B, underwater B/A. Select one-second profile summaries
at 12-22 s after the first startup log and take each run's median, then the
median of its three repetitions. Animated actors use elapsed time; end-of-run
captures occur at different elapsed times and illustrate the views rather
than serving as timing samples.

RTX 3090 (Vulkan, NVIDIA 617.42), i9-12900K, High/SMAA/GTAO High:

| View | GPU off/on (ms) | FPS off/on | Paired GPU delta range (ms) |
|---|---|---|---|
| Shore | 2.705 / 3.200 | 344.15 / 298.95 | +0.40 to +0.77 |
| Underwater | 3.100 / 3.870 | 303.80 / 251.35 | +0.09 to +0.81 |

Twelve runs supply 119 selected intervals; normalized profile settings differ
only in the master, with zero warnings/errors/panics. The 500 FPS ceiling is
not reached by a selected interval, but pacing is still configured. Real
Windows `Process.TotalProcessorTime` counters sampled around 12-22 s in
repeats 2/3, normalized by mean logged FPS, give approximate total CPU cost
across all threads: shore 4.20/5.55 ms per frame and underwater 4.05/5.32 ms
off/on. This cumulative parallel CPU time cannot be added to GPU time.
`.scratch/water-ab-20261018/results.json` contains the protocol, binary SHA256,
source revision, per-run values and paired deltas; `comparison.csv` and
`runs.csv` accompany raw samples/logs/captures. Both camera pairs were inspected.
Shared-desktop load and animated-scene variation remain visible in the ranges;
this small local scene is not a prediction for a busy grid region or another
water quality. No production code or human grid session changed.

Offline water actions (2026-10-17): combine `AURORA_DEMO=1`,
`AURORA_DEMO_WATER=high`, `AURORA_DEMO_WATER_ACTIONS=dive` and
`AURORA_DEMO_WATER_ANIMS=<folder>`. The folder must contain the twelve numbered
`.anim` files listed in `ao/water_demo.rs`. They are decoded once into memory;
no inventory item, upload or animation import is created. The thirteenth clip
is the authored `assets/animations/swimming/cannonball.anim`, also demo-only.
The ephemeral AO catalogue disappears on exit, and demo settings are not saved.
`dive` jumps and presses descent at 0.20 s to engage the deliberate plunge;
`jump` follows the same trajectory without descent and must retain its ordinary
jump/fall animations, with no intentional entry burst. A second descent press
after an engaged dive switches to the cannonball. The early gesture window is
450 ms from the observed jump signal; a later press remains a cannonball.
`cannonball` presses the resolved descend control once in midair;
`cannonball-flight` starts in flight above deep water, with no jump or predicted
dive, then taps descent to release flight and tuck. `water-walk` and `water-run`
hold the synthetic root with its feet at the native waterline, choose an
ordinary AO gait and highlight the bottom-toolbar walking toggle. Raw falling
signals deliberately exercise local/observer suppression. These paths are
prescribed animation fixtures, not a Havok or bridge physics simulation.
`water-crouch` holds Descend at rest; `water-crouch-walk` holds it while moving
at the 1.2 m/s crouch target. Both keep the native waterline root and use
ephemeral local crouch clips through the ordinary AO owner. Inspect the pose,
wave following and AO command trace; no cannonball or swim transition should
occur. These modes do not change inventory or simulate bridge physics.
`water-jump`, `water-jump-run` and `water-jump-hold` add a prescribed 2.5-metre,
1.25-second arc from the waterline, respectively stationary, running and with
ascent held after landing. Capture frames 765/810/840 to inspect ascent,
descent and restored surface support. These are animation/command fixtures,
with the jump camera pulled back to keep the entire increased arc in view,
not a Havok simulation. For simulator regression checks with bridge 1.32: enable
walking on water, tap the configured ascent key at rest, while walking,
running and crouching, then hold it through landing. Expect one jump, ordinary
AO jump animation retained through descent and a return onto the water without entering flight
or swimming. Repeat in a no-fly parcel, with remapped keys and AO off; check
ceilings/walls, mode disable, seating and bridge loss during the jump.
In particular, Falling must stay hidden before water contact, while disabling
water walking in midair must restore ordinary falling.

Water-walk jump validation (2026-10-23): bridge 1.32 targets 2.5 m / 1.25 s,
selected by the human after simulator tests. Movement keeps the same Jumping
signal and animation clock through descent until actual water contact.
Full `scripts/check.ps1` green: 4,587 passed, zero failures, 27 ignored;
scripts, formatting and Clippy clean. Both LSL scripts lint without errors or
warnings. Regressions cover delayed ascent updates, airborne repress and held
keys, all movement directions, run/crouch precedence, no-fly support, underwater
rejection, context cancellation, timeout and pending-request supersession. An
integrated movement/AO regression injects native Falling throughout the arc,
with replacements on and off; it checks local suppression, observer stop
commands, surface recovery and ordinary fall restoration after disabling support.

Default dev executable `0.7.4+21.gd6129a768` built; three final offline runs
exit 0 without warnings/errors. Nine captures were inspected:
`balanced-water-jump-run` at 1275/1305/1328/1350/1380,
`balanced-water-jump-hold` at 788/840/1010 and neighbouring crouch-walk at 760.
The entire arc stays in view and held ascent leaves the avatar supported after
landing. Profile samples remain at the configured 60 FPS cap with shared-desktop
scheduling spikes; these are prescribed presentation fixtures, not an isolated
performance comparison or simulator physics test. The human validated the final
height and real animation continuity in the normal-mode test viewer and
authorized merge on 2026-10-23. Earlier FR/EN Movement captures also verified
the translated jump hints and configured-key display.

`water-walk-ui` and `swim-ui` also open the Movement tab for EN/FR control
inspection, with only their corresponding water mode active.
`cannonball-late` presses it shortly before contact and checks that even a
pre-loop tuck keeps its complete recovery; `cancel`
switches to flight before impact, `dry` rejects shallow/dry landing and
`missing` omits the optional jump clips while retaining the eight-slot swim gate.
`slots` opens the native AO on the new optional cannonball slot.
`wade` previews a shallow walking gait; `swim` previews the supplied breaststroke
near the surface. `wade-run` raises the shallow trajectory's speed;
`swim-turn` follows a curved path with tangent-facing orientation, and
`swim-cycle` cycles the surface list through the supplied breaststroke, crawl
and backstroke clips without selecting a separate locomotion style.
`swim-submerged` starts below the surface and selects the independent underwater
forward state. `swim-depth` swims at the surface, descends after 1.5 seconds,
swims underwater, then begins a prescribed 0.7-second ascent after 4.5 seconds
using the ordinary resolved descend/ascend controls. `swim-depth-hold` keeps
ascent held after arrival: the pose must switch to surface forward swimming
and remain there, without flight or a release jump. Vertical velocity matches
this synthetic path; its timing is not a measurement of the bridge physics. Inspect local continuity and the offline AO command
trace; actual observer visibility still requires a human grid test.
`slots-surface` and `slots-underwater` show the generic AO list editor for the
two visible swimming states (Add, Combine, tracks and cycle controls).
Former crawl/backstroke assignments are consolidated only through the normal
acknowledged inventory edit path. Original animation items, foreign contents and
source folders are preserved. Verify groups/tracks/cycles in both water lists,
surface-list fallback when underwater is empty, AO disable/pause, scripted
seats, teleports and another viewer observing the selected clips, including
clips whose priority is lower than the original system animation.
For ground-like water crouching (bridge 1.27), let the viewer complete its
ordinary bridge update, enable water walking and hold the configured Descend
key: idle must crouch and forward/back/strafe must crouch-walk without losing
support. Always-run must stay slower while crouched. Releasing Descend restores
the ordinary gait; ascent still jumps, and the toolbar toggle still disables
the mode. Test with an active AO set's crouch slots, disabled AO and missing
slots (standard fallbacks), a nearby observer, waves, shallow shores, a deck,
seats, flight, focus changes and bridge detach/reconnect. The simulator still
owns collision geometry: this adds supported poses, not a physical water floor.
Offline validation (2026-10-18): full `scripts/check.ps1` green, 3,574 passed,
zero failures, 26 ignored; scripts/formatting/Clippy clean. Built the default dev
viewer. Inspected `.scratch/water-crouch-idle-fr.png`, `water-crouch-walk-fr.png`,
`water-crouch-controls-fr.png` and `water-crouch-controls-small-en.png`, compared
with `water-crouch-before-walk.png`. Distinct synthetic crouch assets start in
the AO trace without a water impact, and the new hints fit both locales. The
offline poses are basic stand-ins; real authored animations and simulator
physics still require the human online test. Last-five before/idle/moving
medians: 60 FPS, agent-camera 0.10/0.10/0.09 ms, UI 0.51/0.44/0.46 ms and GPU
2.68/2.66/2.63 ms; shared load and pose changes prevent an isolated speed claim.
Owned logs contain zero errors/panics, no action-fixture warnings and only the
baseline synthetic outfit warning in the two UI captures.
`swim-stop` stops at 1.5 seconds so the old wake must spread/settle without a
new trail. `swim-float` holds the physical root still and selects the supplied
surface-floating clip: inspect the local visual heave through a crest and
trough, including Low and the effect-switch cycle. `swim-float-offset` and
`swim-offset` shift the synthetic simulator root 40 cm above its target after
one second, without moving that target: both idle floating and translating
breaststroke must keep following waves. This reproduces the former 30 cm
activation cut-off. `swim-crawl` and `swim-back` choose entries in the same surface list;
fast/backward input does not create separate AO style states. The body, attachments and
following camera should share the smooth height change; camera inspection
must not stop it. The saved float height stays additive. Disabling waves and
leaving the surface settle the offset; diving, seating, teleporting and
arrival changes must not keep a stale offset. Wave-carried motion alone must
not generate splash or bubble streams. `water_contacts=trace` also records
offline physical height, sampled wave displacement and filtered visual heave.
It includes the resolved AO swim intent; root drift, unavailable terrain or
motor readiness and measured speed must not veto an established surface pose.
Add `:no-wake` to the water quality for the matching interaction-off
performance/visual comparison. All use the same ordinary contact/GPU paths.
The original `wade` and `swim` modes move the local avatar along a repeatable nine-second
path, without a jump. Compare the direction of the low lateral splash fans,
the small detached droplets, and the separate local trails extending behind
the immersed limbs: a smoother central depression, expanding lateral crests,
secondary ripples and thin broken foam rather than a painted white carpet.
Turns must preserve the previous path; stops must let the depression refill
and the waves decay. Repeat with spray/bubbles and contact foam off: water
relief/reflection/refraction must remain, including with ambient waves off.
The GPU atlas regression executes the production shader and checks the
negative trough, positive outgoing shoulders, finite bounded slopes, no foam
when disabled, and clearing for interaction-off, dry land or mismatched water
height. Swept-contact tests compare path length at 30/60/144 fps and guard
turn/stop lifetimes. Stationary
turnarounds must not produce a fountain. The anatomical contact emitters remain
active with flat custom poses, while deep limbs cannot throw surface spray.
Horizontal joint centers just below water still displace the surface with
depth attenuation; fully dry or deeply submerged poses produce no body wake.
The synthetic jump begins at frame 750 and repeats every 540 frames. Its path
is a local trajectory fixture, not a Havok simulation. Native AO selection,
AgentAnimation messages, bridge acknowledgements, skeletal sampling and GPU
water events use their ordinary paths. Inspect the held air pose, a single
body-contact splash, the P5 entry hold and its full 2.4 s recovery, and the
return to swimming. Missing clips or cancelling must not make a large splash.
The bridge observer reports armed/airborne/water contact without taking over
native jump controls or applying force. Simulator collision and other-viewer
visibility still require later human grid validation.

The twelve supplied clips are surface idle, breaststroke, crawl, backstroke,
underwater idle, down, up, inertial entry, flight exit, shore exit, dive jump
and dive entry. Cannonball is an additional optional clip. The supplied strokes are entries
in one surface list; underwater forward swimming has a separate list. Existing
eight-state sets use their surface list until underwater clips are assigned. A fresh
PageDown (or the rebound descend key) during a jump above known deep native
region water requests a cannonball. Held crouch on the ground, typing, flight,
locked/seated avatars, teleports and unknown/shallow landings cannot trigger it.
No generic prim/mesh water detection is inferred.

Cannonball rendering uses ten radial packets; dives use six smaller packets.
Each spends two instances on connected, torn water-sheet patches and leaves
quality-scaled remaining droplets even at Low quality. Thin sheets refract the
background, reflect the sky and break up along their foamy rim. An expanding
foam ring follows the falling crown; one small air packet accompanies entry.
Walking and swimming add low directed fans rather than radial fountains.
All reuse the water particle pipeline and shared
particle budget; no texture, extra binding, render pass or CPU fluid solver is
added. Splashes, bubbles and displacement remain independently switchable.
Completed displacement impulses expire after 0.3 s; ordinary droplets/fans
after 0.8 s; entry sheets/rings after 1.6 s; bubbles after 3 s. At the quality
event cap, active entry sheets/rings take priority over ordinary contacts.
Low quality must preserve a complete crown while reducing detached droplets.
The rough underwater reflection fallback now stays continuous when micro-normals
turn a ray back toward the underside; scene-buffer coverage remains approximate.

Offline acceptance (2026-10-17): full `check.ps1` passes (3,416 passed, 26 ignored),
with clean scripts, formatting and workspace/all-target Clippy. LSL lint has zero
errors/warnings; native WGSL specialization and the nine final action/UI captures
have no GPU error or panic. The late-PageDown capture broadcasts dive -> cannonball,
makes one impact, then waits 2.716 s after the stop before surface idle; the P5
entry capture preserves its full recovery. Cancellation, actual dry terrain and
missing optional clips make no intentional impact. FR/EN AO fields and the FR
Movement checkbox are readable. The older `AURORA_DEMO_AO=movement` fixture used
for that one UI capture reports its synthetic missing Current Outfit folder; it
does not affect the new pack decoder or water action tests. Six additional baked
captures inspect the tuck/crown, running shallows, continuous underside reflection
and every effect toggle across the four quality levels. On the shared RTX 3090,
1440 x 900 High/SMAA/GTAO High, adjacent pre-impact/impact one-second intervals
both report 1.65 ms total GPU and 0.235/0.236 ms surface water pass; agent/camera
work is 0.09/0.10 ms. These simple-scene intervals are illustrative only, and the
water-pass marker excludes compute/opaque caustics. No online test, import or
upload was performed. All twelve source hashes remain unchanged.

Water walking / airborne cannonball acceptance (2026-10-17): enable the small
walking icon in the bottom bar, then walk/run from shore onto water and stop.
Its violet active state must match the Movement switch. With Aurora AO active,
verify ordinary Standing/Walking/Running selections, groups and cycles; with AO
disabled or an empty slot, verify stock motions. Repeat with an observing viewer.
Descent leaves water walking without enabling swimming. A tap of ascent
underwater with swimming enabled must return promptly and settle into
the surface swim state; held ascent begun underwater must stay in that state.
In flight above water, tap descent without any preceding jump: flight must
release, the optional assigned cannonball clip must play and actual water
contact must produce one entry splash. Repeat from a long fall, with no dive
slots assigned, with AO disabled and with the cannonball slot absent. Resuming
flight, sitting, teleporting or bridge loss must cancel. Keep ordinary shore
walking, land running, platform collisions and Movelock behavior intact. These
physical/remote checks require human grid validation; no automatic login.

Offline verification: full `scripts/check.ps1` green (3,517 passed, zero failures,
26 ignored); LSL lint clean. Native water walking/running, flight tuck/contact,
the EN/FR Movement tab and adjacent wading captures inspected without warning,
error or panic. The flight fixture starts without a jump and emits exactly one
actual-contact burst. At 1440 x 900 High/SMAA/GTAO High, final reference/updated
wading intervals both sustain 60 FPS; CPU agent/camera is 0.12–0.18/0.10 ms,
GPU 2.30–2.38/2.14–2.21 ms and updated water pass 0.288–0.299 ms. Earlier
shared-desktop GPU intervals varied up to 6.46 ms: do not infer a speedup from
these samples. No added render pass or shader. Native trajectories are prescribed;
real bridge responsiveness, collisions and remote playback remain human checks.

Bridge 1.25 response checks: after its confirmed surface capture, tap descent
briefly while stationary and moving. It must begin descending with the animation
without the extra HTTP round trip, keep its depth after release and accept ascent
as cancellation. Repeat while a periodic bridge refresh is pending and with
Movelock enabled. With bridge 1.26, tap ascent once in surface mode to jump
toward a reachable platform, whether stationary or moving. The feet must clear its lip before
crossing, and flight must clear on contact with its top, including after a slow
crossing or stale native Falling/foot-plane updates. Repeat when swimming began
in flight. Holding that same press on land must not enable AutomaticFly;
release and a fresh ground press retain the ordinary flight gesture. Two distinct
surface ascent presses within 400 ms request flight with feet about 1.25 m above
native water. The second must replace the first jump motor and its landing target,
even during its Leaving phase. Repeat while stationary and moving. Holding one
press, key repeat, a second press after the window, descent, seating, suspension,
bridge loss or returning from underwater must not accidentally start flight.
Respect no-fly parcels; underwater ascent remains surface return even when held
through arrival. Jump from dry land
toward water, then press descent early to dive; jumping alone must not select the dive automatically.
Keep the independent cannonball available from later jumps and from flight.

Bridge 1.26 acceptance: try a cannonball while hovering over a solid pontoon,
including overlapping its edge: it must not tuck or turn flight off. Move clear
over the water and tap descent again: tuck and the real-contact splash return.
Phantom decorations and fully submerged floors do not block arming. Toggle the
swim and water-walk toolbar icons in both orders; an inactive click enables only
that mode, an active click leaves both off. Repeat through the Movement switches,
with run/flight assistance enabled (those must not light the swim icon), and with
an incomplete selected swim AO (swim stays off and the working water-walk mode
is preserved). Explicit Fly after an exit must remain available. These are human
grid checks: local tests use rendered geometry, not the simulator's collision mesh.

Response follow-up offline validation: full `scripts/check.ps1` green (3,524
passed, zero failures, 26 ignored); LSL lint clean. The actual scalar LSL native
edge and stale-refresh guard pass with mocked native calls, including the
unchanged watchdog and explicit cancellation. Native ordinary jump, deliberate
dive and its contact, flight cannonball, adjacent depth motion and updated French
Movement help inspected without warning/error/panic. Dive and cannonball produce
one intentional burst each. The 1440 x 900 High/SMAA/GTAO High depth comparison
stays at 60 FPS; reference/updated CPU agent-camera 0.16/0.12 ms, GPU 2.11/2.14 ms
and water 0.379/0.358 ms. These are shared-desktop samples with animated phases,
not isolated performance gains or a Havok latency measurement. The human tests
real onset and platform arrival on the grid; no automated login or animation upload.


Water ledges / exclusive-mode offline validation (2026-10-17): full
`scripts/check.ps1` green, 3,529 passed, zero failures, 26 ignored; LSL lint
clean. Geometry tests include a pontoon edge under the body, clear water beside
it, phantom exclusion, submerged floors and support independent of native Falling.
Movement tests cover immediate surface key edges, underwater hold continuity,
slow crossing, repeated stale flight flags after arrival, low platform tops for
small avatars, explicit Fly and fresh ground AutomaticFly, both-off mode toggles
and the core AO gate. Extracted actual LSL shore goal selection passes vertical
clearance, horizontal crossing, settling and release; an ideal first-order motor
reaches the fixture in 1.20 s, which does not establish Havok timing. Native
French water-walk and English swim controls, adjacent depth motion and flight
cannonball captures inspected without warning/error/panic. The cannonball
fixture produces one intentional contact burst. At 1440 x 900 High/SMAA/GTAO
High, reference/updated depth intervals both sustain 60 FPS; GPU 2.11/2.11 ms,
water 0.370/0.362 ms, agent-camera 0.15/0.10 ms. Shared desktop load and pose/wave
phase vary: these samples do not establish a speedup. No renderer change. Real
collisions, arrival flight state and remote playback remain human grid checks.


Single/double surface ascent follow-up (2026-10-17): full `scripts/check.ps1`
green, 3,530 passed, zero failures, 26 ignored; formatting and Clippy clean.
Regressions cover a stationary single ledge jump, the second tap replacing an
active shore/fallback motor within 400 ms, no repeated action while an input is held,
late/disabled/seated/suspended/grounded/descent interruptions, no-fly parcels,
underwater return continuity and native/enhanced flight handover. Flight retains
the higher existing exit target. The script/protocol remain at bridge 1.26; this
change only classifies viewer input. Repeat the surface gesture at rest and
while moving on the grid; the human performs collision and observer validation.


Underwater camera immersion (2026-10-18): the existing absorption control
(Preferences > Aurora > Water, depth colour and underwater haze) now enables
RGB extinction, EEP-coloured scattering and progressive depth lighting while
the camera is submerged. Emissive/fullbright sources lose their warm wavelengths
along the view path, without receiving a second sunlight-depth attenuation.
Enter/leave near the surface, look at objects at several distances, swim deeper,
look up through the exterior window, then test a night/custom EEP water/sky.
Repeat at Low quality, with absorption disabled, and with the master disabled;
above-water lighting and sky must remain unchanged. No new pass, texture, GPU
readback or simulation allocation. The inexpensive medium model is identical
across qualities; existing surface, reflection and particle detail still scale.

Offline checks: full `scripts/check.ps1` green (3,530 passed, zero failures,
26 ignored). The actual production-WGSL GPU test samples air-boundary clipping,
RGB attenuation, deeper paths, zero path/density, near-surface fade, night
scattering and Legacy scalar transmission when disabled/above water. Six
source-shader previews inspected without warning/error/panic: reference fog,
enhanced underwater, upward window, midnight, Low and adjacent surface MSAA.
At 1440 x 900 High/SMAA/GTAO High, reference/enhanced underwater intervals both
sustain 60 FPS, GPU 1.98/1.94 ms and water 0.211/0.202 ms. Shared desktop load
and animated geometry/waves vary; these samples do not establish a speedup.
Plain water inspection cameras now remain fixed across normal camera updates;
free camera and animation-action framing are retained. `:underwater-deep` uses
a lower lagoon view above its seabed, `:no-absorption` compares the previous
scalar fog at the same camera position. Human grid acceptance remains pending.

Wave crossings: `high:waterline-crest` places the offline camera 5 cm above
native water under a moving crest; `high:waterline-trough` places it 5 cm below
native water in an exposed trough. These inspection views choose a nearby
crest/trough each frame and disable interactions to isolate macro waves.
These waterline views omit the moving hull so it cannot obscure the interface.
The frame-900 demo log records camera and sampled surface heights. The crest
must show underwater fog across all exposed seabed pixels, including below the
near-clipped surface; the trough must show the ordinary above-water view.
Also check `low:waterline-crest`, `high:waterline-crest:no-waves` and
`off:waterline-crest`, ordinary underwater/window/surface views, and MSAA/TAA.
Camera immersion uses the shore-damped inverted Gerstner surface and the
current GPU wake height near the waterline. Outside the possible wake crossing
band, classification needs no field fetch; its small fog-boundary adjustment
fades away by 38 cm of separation. Coloured attenuation now follows metres
travelled in water, with no camera-depth fade under shallow crests. The physical
water plane and swimming controls stay unchanged. Full `scripts/check.ps1`:
3,531 passed, zero failures, 26 ignored. Reference/optimized underwater samples
at 1440 x 900 High/SMAA/GTAO High: median GPU 2.22/2.44 ms and water
0.161/0.214 ms across the last three intervals; shared desktop load and moving
geometry vary, so these are observations rather than an isolated cost estimate.
Human grid check: slowly cross crests and troughs with the camera while
floating/swimming, then disable waves/master and repeat.

Near-plane crossings: `high:waterline-split:no-waves` holds the eye 1 cm above
flat water with a horizontal view; the near rectangle crosses the interface.
The exposed lower strip must use the same water/refraction as the rest of the
surface, rather than showing a clear unfiltered seabed. `high:waterline-split`
follows the same crossing on waves; append `-under` to `waterline-split` to
inspect the inverse crossing from 1 cm below the surface. Inspect SMAA, TAA,
MSAA, Low, disabled absorption and master-off variants. Fully emerged/deep
views must retain correct coverage and opaque-object occlusion.

The geometric revision replaces the procedural near-plane patch with the
actual displaced mesh. Hardware depth clamping retains its real perspective
and coverage through the near plane. Adapters without depth-clip control use
a portable pipeline only near water: the vertex shader retains real XY/W,
and the fragment shader restores the actual clamped depth for occlusion.
Append `:portable-depth` to any water demo to force this fallback at device
creation; it has no effect outside demo mode. The demo log identifies the
selected path. A separate single-sample depth target selects the closest real
interface before water shading. Its monotonic view-distance depth preserves
ordering even for two surfaces before the normal near plane. This prevents
distant top faces from overwriting a nearby underside; the ordinary scene
depth stays intact for submerged blended objects. The prepass runs only with
displaced waves/interactions and reuses the existing indirect water draws.
At 1440 x 900 its target is about 4.9 MiB. This interface prepass adds no scene
colour render, copy or readback; GPU Water timing includes it. The exterior
capture described below has its own reduced colour/depth target.

The interface side comes from its actual geometric tangent. Transmission and
reflection use known submerged segments without clipping them a second time
to a fictitious flat surface. Coloured attenuation depends on water distance,
including millimetre-deep camera positions. Surface/underwater paths share
the same extinction model; source water fog is removed before applying the
new path once. Immersed opaque endpoints retain their partial water path
even with the eye just above the local surface; all-air rays remain clear.
This avoids unfiltered holes at the analytic/triangulated wave boundary and
keeps attenuation continuous at the exact waterline. Submerged sunlight
attenuation depends on object depth, and
the classic lighting model stays stable when the camera crosses the surface.
Fullbright/emissive sources retain their own emission. Sky and EEP parameters
are unchanged. Existing absorption/Fresnel/master controls remain independent.

The production vertex/fragment GPU test renders both sides, fully clear/deep
cameras, roll and front/behind opaque depths on native and portable paths at
1x/4x MSAA. The real interface depth shader also renders overlapping sheets
in both draw orders, before and beyond the near plane, retaining the closest
sheet. Production fog tests cover actual interface segments, air clipping,
RGB extinction, eye positions 1 mm above/below the surface, partial entry
from air, entirely dry rays, zero path/density, disabled absorption, night,
crests/troughs and GPU wake entry. Full `scripts/check.ps1`: 3,532 passed,
zero failures, 26 ignored; formatting and Clippy clean. Eight baked native
captures cover flat/wavy crossings, portable Low/MSAA, TAA underside, upward
window, adjacent surface, deep water and disabled master/absorption; all exit
0 without warnings/errors/panics. High/SMAA/GTAO High at 1440 x 900 stays at
60 FPS. The deep-view last-three water medians before/after nearest-interface
selection are 0.269/0.273 ms; total GPU medians are both 1.89 ms. Shared-desktop
spikes and moving geometry vary, so these observations do not isolate cost.
Human grid acceptance: reproduce the screenshot at the waterline, tilt/roll
the camera and move slowly through crests/troughs, then check the upward
window and deeper underwater view. No clear unfiltered strip should open
below the near-clipped water surface.

Exterior optics: `high:optics-60:no-waves` fixes the camera, animation time and
four coloured air reference panels. The old main-screen-only refraction loses
three panels at this angle because they are outside the main frustum. Append
`:screen-only` to reproduce that baseline in the same binary, or `:air-quarter`
to compare quarter-resolution exterior capture with High's half resolution.
`optics-<degrees>` accepts upward pitches from 0 to 89 degrees; inspect 20, 45,
60 and 89, omit `no-waves` for the same frozen wave phase, and append `:near-eye`
for an eye 1.5 cm below the flat region plane. All these tokens are demo-only.
The existing `:no-fresnel` token also disables the auxiliary capture and keeps
the baseline transparent material. Main camera FOV, sky and EEP are unchanged.

At 60 degrees upward the flat interface should transmit most of the exterior;
at grazing angles, total internal reflection of the submerged scene is
expected. The production GPU dielectric probe checks that distinction rather
than weakening reflections arbitrarily. Inspect panel/shoreline continuity,
wave distortion and the waterline variants above, including Low/MSAA and
portable depth. The auxiliary view uses quarter/three-eighths/half/three-quarters
linear resolution at Low/Medium/High/Ultra, only while an underwater exterior
window can be visible. It captures opaque/masked air geometry with stored
depth, reuses reflection draw candidates and existing pipelines, and does not
rerender shadows or GTAO. High's extra colour/depth target is about 3.7 MiB at
1440 x 900; GPU Water timing includes the capture. Transparent exterior layers
and single-depth disocclusions remain approximate. Use identical frozen
`optics-*` views for timing comparisons, then validate the moving ordinary
`window`, `underwater`, `surface` and waterline views.

Verification (2026-10-18): full `scripts/check.ps1` green, 3,536 passed, zero
failures, 26 ignored; the separately run GPU culling comparison also passes
with distinct reflection/exterior frusta. Production GPU tests distinguish
total internal reflection at grazing angles from strong upward transmission;
the capture-camera test aligns the central interface exit with its refracted
axis at 45/60/89 degrees. Baked fixed-camera images recover all four air panels
where screen-only loses red/green completely. Final RTX 3090/Vulkan at
1440 x 900 High/SMAA/GTAO High, last-five-interval medians for
screen-only/half/quarter exterior are Water 0.151/0.155/0.128 ms and total GPU
1.16/1.08/1.17 ms, all at the 60 FPS cap. CPU parameter work is
0.10/0.09/0.10 ms. Shared desktop load varies; these local simple-scene
observations do not establish a general speedup or a busy-grid cost.

### Received inventory filing

`AURORA_DEMO_INVENTORY=received-filing` exercises new direct deliveries, an offline delivery, an intact folder and an accepted scripted gift below their sender/owner legacy names. `aurora-received-filing` shows the same fixture in Aurora's wide inventory. Both use the real filing state machine and synthetic inventory acknowledgements; at frame 700 the Received tree expands for capture. `aurora-received-unread` and `aurora-received-unread-compact` leave the Received tab unopened to show the combined red badge in the sidebar and compact tabs. Use `AURORA_CAPTURE_FRAMES=900`. Preferences › Inventory (`AURORA_DEMO_OPTIONS=inventory`) shows the OFF-by-default switch with the shared experimental icon; there is no warning paragraph below it. No grid access or old-inventory classification. On the grid, the human checks enabled/disabled direct and object-owner deliveries, offline retrieval, rejection, folder contents, and Marketplace purchases above the sender folders. Check another gift from the same sender raises the red badge after opening Received, that type destinations never override sender filing, and that empty sender folders move to Trash (MarketPlace and nonempty/unloaded folders stay).

Received items are the last group of Preferences › Inventory; Aurora's water is on Graphics › Reflections and water (`AURORA_DEMO_OPTIONS=graphics/3:cogs`).

Unread state is account-specific and survives reconnecting, restarting and clearing the inventory cache. On first use, existing history establishes a baseline without a red total. Check that reading Received and reconnecting shows no red badge, a new offline gift counts once, and an unread gift stays unread after restart. Automated regressions use real isolated state files and fresh inventory models, including old creation dates, late folder metadata, arrivals during initialization and a read before disk loading completes.

### Anti-spam validation (#678)

`AURORA_DEMO_ANTI_SPAM=1` opens Preferences > Protections and enables the disabled-by-default protection for this demo run. At frame 700 it injects 15 script prompts, 15 scripted inventory offers, 15 teleport offers, 65 sounds and 65 sound preloads. Capture after frame 900. Modes `off` and `global` disable protection or combine the categories. Mode `text` sets the event allowance to 2, newline allowance to 5 and includes own objects: four own scripted gifts admit two notifications, then a six-LF object chat and a six-LF resident IM are suppressed. It lists three IM blocks (own gift owner, chat object and resident), with one notice each. Use both `--language en` and `--language fr` and compare `AURORA_DEMO_OPTIONS=protections` without this fixture for the normal disabled state.

With protection enabled, ten prompts/offers of each source and sixty sounds/preloads pass; the next event blocks that source/category for the session and produces only one nearby-chat notice. Preferences lists five blocked pairs. The two sound queues stay independent. In global mode the sound and preload counts combine. Reset clears all counters/blocks; Unblock removes one listed source/category. Changing a setting clears the session counters and blocks. Existing accepted notifications stay available. Typing, saved text IMs, self/nil sources, known own objects (unless included), collision sounds and sound-stop commands are exempt. The object and its owner are both checked for sounds, preventing rotation of object UUIDs from bypassing the owner limit. A bounded source table falls back to a shared overflow counter at 8,192 active pairs rather than allocating without limit; reset also clears that fallback.

Nearby chat has its own event queue. The single-message limit defaults to 70 LF characters (5–1,000 in Preferences), following checkNewlineFlood; the first line and visual wrapping do not count, so exactly 70 LF passes and 71 blocks. This check also covers saved text IMs, independently of their rate exemption. Both chat and IM newline floods create a persistent IM block, as in the Legacy Viewer; later chat checks consult it too. With own objects excluded, chat newline checks aggregate by owner; with them included they use the speaking source. System/self text, group invitations and IMs from known own objects retain the reference exemptions. Script/debug and internal bridge traffic keep their existing routing.

Human validation: enable the protection, exercise ordinary inventory/teleport offers and script dialogs, and confirm they still work. Repeated deliveries from one test source must stop at the threshold while another source still works. Set 2 events / 30 seconds and Include my own objects, then run four llGiveInventory calls from one owned cube: two notifications and sounds pass, the third blocks, and later offers stay suppressed. TASK_INVENTORY_OFFERED carries the owner UUID, not the object UUID: multiple cubes owned by the same resident share this counter. Deliberate improvement over the Legacy Viewer's blanket self exemption: Include my own objects also applies to our scripted gifts, without counting self text. Test per-source unblock, reset, the own-object switch and sound stopping. Test a five-LF nearby message with a limit of 5 (passes), then six LF (suppressed and source blocked); its next short message stays hidden until unblock/reset. IM tests require another resident or their object because known own object IMs are exempt. Repeat with protection off and at the default 70/71 boundary. Grid validation is performed by the human in a controlled setup; no credentials or grid connection are used by automated tests. Incoming inventory/teleport offers use the common IM queue, as in process_improved_im; suppressed offers are dropped without automatic acceptance, decline or inventory mutation. Sound changes carried by full object updates respect existing sound blocks without counting unchanged loop snapshots as new sound events.

## Fuzzing

`fuzz/` feeds arbitrary bytes to the readers of data that any resident or
simulator controls, the way the viewer calls them: JPEG2000 textures
(`j2k`), mesh assets and their sections (`mesh`), animations (`anim`), LLSD
in its three encodings and XML-RPC replies (`llsd`), GLTF materials and
their per-face overrides (`material`), UDP packets with the object and
terrain messages (`packet`), compressed object updates and texture
animations (`object`), texture entries, extra parameters and particle
systems (`prim`), prim shapes from their quantized parameters (`volume`),
sculpt maps of any size (`sculpt`), terrain and wind patches (`terrain`),
sounds (`sound`), notecards, task inventories, gestures, wearables and
avatar base meshes (`misc`). A reader must return an error for any input:
a panic, an input that runs for more than 10 s or an allocation of more
than 1 GB is a bug. The seeds are valid inputs made by our own encoders (or
shipped assets), never content of the grid cache.

`fuzz/` is its own Cargo workspace: the viewer's builds and `check.ps1` do
not compile it. It has no `Cargo.lock` of its own: copy the viewer's first,
so that every shared crate is fuzzed at its shipped version (cargo only adds
`libfuzzer-sys` and drops what `fuzz/` does not use). On this PC (stable
Rust, Windows), `fuzz-local` mutates the seeds without coverage feedback:

```powershell
Copy-Item Cargo.lock fuzz/Cargo.lock
cargo run --release --manifest-path fuzz/Cargo.toml --bin fuzz-local -- all --seconds 60
cargo run --release --manifest-path fuzz/Cargo.toml --bin fuzz-local -- mesh --seconds 300 --seed 7
cargo run --release --manifest-path fuzz/Cargo.toml --bin fuzz-local -- --replay mesh fuzz/artifacts/mesh/crash-…
cargo test --release --manifest-path fuzz/Cargo.toml --lib   # every target reads its seeds
```

A finding goes to `fuzz/artifacts/<target>/` with a `.txt` note (panic
message and location, or the refused allocation); the run ends with code 1.
Fix the reader, add a unit test with the minimized input as bytes, and
replay the file. `--timeout` and `--max-alloc-mb` change the limits.

The Fuzz workflow (`.github/workflows/fuzz.yml`) runs the coverage-guided
version every night on Linux: libFuzzer through `cargo fuzz` on nightly
Rust, with AddressSanitizer (OpenJPEG is C), 10 minutes per target
(`workflow_dispatch` takes another duration). The corpus each target grows
is kept from one run to the next; a failing input is uploaded as the job's
artifact. Locally on Linux:
`cargo +nightly fuzz run -O mesh --features libfuzzer`. Pull
requests that change `fuzz/` only check that every target reads its seeds.

### Lost inventory recovery

`AURORA_DEMO_INVENTORY=lost` opens Lost And Found with three synthetic originals:
a null-parent orphan, a simulator return using the categorization sentinel, and
an item with a missing parent. A dangling folder and its child exercise startup
hierarchy repair. The normal background orphan request receives a synthetic reply;
all filing uses the production model and reliable-move planner. Capture at frame
900. Originals keep their UUIDs and permissions; recovery does not attach them.
Check ordinary `AURORA_DEMO_INVENTORY=1` and the existing attachment regressions
alongside it. Online validation is done by the human: check returned objects,
startup orphan retrieval and ordinary script attachments remaining in Objects or
their original folder. No credentials or grid actions are automated.

`AURORA_DEMO_XR_DEVICE`: Linux/Windows/macOS offline demo only. With the active OpenXR runtime running and the headset connected, initializes the actual viewer graphics device through OpenXR Vulkan-enable2, requesting Aurora's normal bindless and rendering features. Desktop output remains on the window; combine with `AURORA_DEMO_RENDER_TEXTURE=1` and a capture to validate the full world-rendering output. This does not start an XR session or send the world to the headset. Runtime initialization errors fail this explicit experiment rather than silently testing another device.

`AURORA_DEMO_XR_WORLD`: Linux/Windows/macOS offline experiment only, with an OpenXR runtime running and
the headset connected. Implies the compatible XR device path. After frame 600,
creates independent eye renderers and sends the actual demo scene to the headset
for 15 seconds, then returns to desktop rendering. Eye dimensions are capped at
1600 pixels per axis for bring-up. Geometry, textures and asset uploads are
shared; eye depth, visibility, exposure and temporal state are independent. CPU
frustum rejection is disabled in this scenario so turning away from the desktop
camera does not discard those lists; per-eye GPU culling remains active. The
desktop mirror follows the left eye at up to 30 FPS during headset rendering. Manual VR entry uses the bottom-toolbar headset button and Escape exits. This
automatic scenario retains its timed stop for regression testing. Late runtime attachment and production disconnect recovery remain pending. Run with a distinct test title; omit capture
exit while visually inspecting the headset.

VR horizon regression: start with the desktop camera pitched down and the
headset resting tilted, then wear it and look physically level. The view must
face the horizon; looking up/down and rolling the head must still work. Escape
restores the original desktop pitch. Verify the desktop mirror follows the eye
without stretching, exposes no viewer UI, and returns to the normal interface on
exit. Repeat after resizing the desktop window. The mirror is capped at 30 FPS
and does not render a separate spectator camera.

`AURORA_OPENXR_LOADER`: macOS only. Optional absolute path to the installed
Khronos `libopenxr_loader.dylib`; otherwise standard dynamic-loader discovery is
used. The loader and headset runtime are installed separately. Runtime selection
is still performed by OpenXR (`XR_RUNTIME_JSON` may select a development manifest).
This is also accepted during normal viewer startup; no loader is downloaded or
bundled. See [VR setup](VR.md) for platform requirements and validation status.
