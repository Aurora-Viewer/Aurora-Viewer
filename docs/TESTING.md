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
[Profiling](#profiling)

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
| `AURORA_DEMO_POS="x,y"` | Start position |
| `AURORA_DEMO_TOD=0..4` | Time of day |
| `AURORA_DEMO_ALTITUDE=<m>` | Puts the whole demo location at this altitude |
| `AURORA_FPS_LIMIT=<n>` | Frames-per-second cap for tests (10–500) |
| `AURORA_WINDOW_SIZE="w,h"` | Window size for this launch |
| `AURORA_TEST_NEUTRAL_POSE=1` | Local neutral-pose comparison in the online or demo Hover Height floater |
| `AURORA_LOG_NAME=<name>` | Name of the log file |
| `AURORA_EMOJI_FONT=<path>` | Uses another emoji font than the built-in one (PNG bitmaps: CBDT or sbix) |
| `AURORA_MAX_AVATARS`, `AURORA_MAX_COMPLEXITY`, `AURORA_AA`, `AURORA_SHADOWS`, `AURORA_PROJECTOR_SHADOWS` | Override these settings |
| `AURORA_GRAPHICS_PRESET=potato\|low\|medium\|high\|deluxe` | Applies a graphics level before the overrides above |

The demo shows no sample offer or dialog by default. The fountain menu,
gift and teleport offer are restricted to `AURORA_DEMO_NOTIF=1`.
`AURORA_DEMO_DIALOG` adds only the requested dialog;
`AURORA_DEMO_INVENTORY=offer|offer-reject` adds only the received item.

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
Body attachment, shape, physical-size and newly available asset changes can
invalidate the estimate; an initial size-only estimate is upgraded once a
usable contact plane arrives. Changing the offset reuses the cached result.

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
| `AURORA_CAPTURE=<file.png>` | Saves a screenshot of the frame |
| `AURORA_CAPTURE_FRAMES=<n>` | Frame captured (240 by default) |
| `AURORA_CAPTURE_EXIT=1` | Quits after the screenshot |
| `AURORA_CAPTURE_LAYOUT=1` | The screenshot reloads the saved layout (`ui_layout.ron`) |
| `AURORA_DEMO_SNAPSHOT=<scenario>` | Photo window, full-frame view, a photo saved at its size, sent to the inventory or posted to the profile |
| `AURORA_DEMO_SNAPSHOT_FRAME=<n>` | Frame the scenario's photo is taken (700 by default) |
| `AURORA_DEMO_SNAPSHOT_TILE=<px>` | Largest side drawn in one pass: forces the tiles |
| `AURORA_DEMO_CONTROLS=camera\|movement\|compact\|classic\|free\|free-compact` | The Controls window open on a tab, Firestorm's two windows, or driving the photo window's free camera |

`AURORA_CAPTURE_LAYOUT`: a screenshot otherwise starts from the default
layout, to be reproducible. With this variable it reloads that of the
previous session (without ever writing it): this is what is needed to see
what a saved layout gives in a window of another size (the windows keep
their edge). A layout is only written by a normal exit, for example
`AURORA_DEMO_EXIT=quit` without `AURORA_CAPTURE`.

`AURORA_DEMO_SNAPSHOT`: `open` shows the photo window at frame 600;
`full` the full-frame view, `full-4x5` (or `full-16x9`, `full-9x16`…) with
those proportions, to see the mask and the guide; `save:<W>x<H>[:png|jpg|bmp][:<folder>]`
also takes a photo of that size at `AURORA_DEMO_SNAPSHOT_FRAME` and saves
it (in the photo folder, or `<folder>`). The log gives `photo: <W> x <H>
drawn in <n> part(s), <ms>`, `snapshot <W> x <H>: <MB> of textures … at its
peak` (the video memory of the photo frame) and `photo saved: <file> (<KB>,
written in <ms>)`. Open the file: its framing is the window's (cropped to
its proportions), without the interface unless asked. With
`AURORA_DEMO_SNAPSHOT_TILE=2048`, an 8K photo is drawn in 12 tiles: compare
it with the same photo in one pass, the tiles must not show (the demo's
particles and animations make a few levels of difference everywhere).
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
`dof` opens the window with its depth of field on (f/1.4, focused by hand
at 3.3 m, the own avatar), `full-dof` the full-frame view with the aperture
in its strip: the view, the preview and the photos are blurred, while
Preferences › Graphics › Depth of field stays off.
`quality` opens the window with its "Photo quality" on: the log gives
`photo quality: on (draw distance 512 m, shadows 4, LOD 4, MSAA 8x)` and the saved
settings keep the user's own values (`draw_distance` unchanged in
`settings.json` after the run); a photo taken then logs
`photo quality: details loaded in … ms` and is drawn at twice its size
(`photo: reduced to …`); `save-quality:<W>x<H>…` takes such a photo like
`save`.
`light` opens the window with its quick light set: sunset, exposure 1.6,
the clouds still and the worn lights off (the log gives `photo light: time
of day 3`); the saved `time_of_day` and `exposure` stay the user's.
`classic` shows Firestorm's floater instead of the panel for this run
(Preferences › Aurora › Photo window, not saved): a floating window in the
middle, Refresh and the controls on the left, the preview on the right.

`AURORA_DEMO_CONTROLS` opens the Controls window at frame 600: `camera` on
its Camera tab (joysticks, zoom, the four views, object view, mouselook,
reset, saved positions, the alt-camera keys), `movement` on its Movement tab
(the eight buttons with the keys of the keyboard's layout, walk / run / fly),
`compact` the window minimized (its joysticks and zoom only),
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
| `AURORA_DEMO_HOVER_HEIGHT=<scenario>` | Manual/automatic avatar grounding and camera toggle |
| `AURORA_DEMO_TP=1` or `"x,y,z"` or `remote` | Teleport after loading |
| `AURORA_DEMO_TP_AWAY=1` | Departure out of range of the demo region, then return |
| `AURORA_DEMO_AGENT_JITTER=<mm>` | Small position corrections of our avatar |
| `AURORA_DEMO_KEY=<arrow>` | Holds an arrow key |
| `AURORA_DEMO_SIT=<n>` | Clicks n times on the Sit button |
| `AURORA_DEMO_MMO="x,y[,1\|2]"` | Steering the avatar with the mouse |
| `AURORA_DEMO_DOUBLECLICK=walk\|run\|tp[,x,y]` or `menu` | Double click on the ground, its wolf menu entry |
| `AURORA_DEMO_RCLICK="x,y"` or `tag` | Right click (context menu) |
| `AURORA_DEMO_PIE=1` | Right-click pie menu |
| `AURORA_DEMO_POINTER=<points>` | Moves the pointer over the interface |
| `AURORA_DEMO_HOVER=1` or `"x,y"` | Cursor swept, or resting on a pixel, on hover |
| `AURORA_DEMO_CLICKS=<ms>` | Clicks on the world at a regular rate |
| `AURORA_DEMO_LOOKAT=1\|own[,x,y]\|targets` | Gaze tracking and gaze of another avatar; `own` / `targets`: the look-at crosshairs |
| `AURORA_DEMO_KEYBOARD=<layout>` | Default movement and letter shortcut keys according to the layout |
| `AURORA_DEMO_CHATCMD="<commands>"` | Types commands in the chat bar |

### Details

- **`AURORA_DEMO_HOVER_HEIGHT=panel|<metres>|cycle|camera|unsupported|failure|auto-static|auto-static-off|auto-rigged|auto-contact|auto-offset|auto-cycle`**:
  opens the hover-height floater at frame 600. A number sets the height;
  `cycle` previews +0.4 m at 720, commits at 780 and resets at 960. `camera`
  repeats the cycle with "Affect camera view" enabled; otherwise the camera
  stays at its original height. Capture at 660, 750, 840 and 1020; include
  719, 720 and 721 to catch a camera jump on the first adjusted frame.
  With the toggle off, drag continuously in both directions: the background
  must stay fixed throughout, also when seated or using a scripted camera.
  `unsupported`
  disables adjustment as on a region without AvatarHoverHeightEnabled;
  `failure` simulates a rejected update and exposes Retry. Demo changes stay
  in memory and never contact a grid. Use the slider, its editable value,
  +/- (Alt: 10 cm, Ctrl/Shift: 1 mm, otherwise 1 cm), Reset and the camera
  switch for the manual check. On the grid, also verify other viewers see
  the committed adjustment, then teleport and reconnect to check persistence.
  `auto-static` adds two rigid soles with a linked heel at frame 620;
  the `auto-*` scenarios frame the feet at 600, after the login camera reset.
  Their avatar reports a 1.68 m physics height, matching the offline
  simulation's 0.84 m centre clearance; the generic fixture's 1.9 m size
  does not match that simulation. Grounding uses the simulator-reported
  contact distance when available, otherwise the physical height, not the
  independently computed shaped skeleton height.
  `auto-static-off` repeats it with automatic adjustment disabled for comparison;
  `auto-rigged` adds a rigged shoe mesh on the spine with an unrelated low
  triangle whose foot weights are zero (it must not lift the avatar).
  `auto-contact` uses those shoes with a reported 1.48 m physical height and
  a contact plane at the actual ground: the 0.84 m centre clearance includes
  a 10 cm margin over half that size. It must give the same grounded result
  as `auto-rigged`, not raise the shoes by those extra 10 cm. Enable
  `AURORA_TEST_NEUTRAL_POSE=1` to compare the exact neutral pose.
  `auto-offset` uses that same contact-plane fixture: +0.05 m offset at 700
  gives +0.316 m total height; -0.03 m at 860 gives +0.236 m. Removing the
  shoes at 940 leaves the fixed -0.03 m offset. Capture 660, 840, 920 and 1100.
  `auto-cycle` adds the rigid shoes at 620 and removes them at 940; capture
  580, 840 and 1100 to compare barefoot / soles on the ground / barefoot.
  Automatic adjustment is enabled by default in Hover Height. Manual edits
  last until the next body attachment change; HUD changes must leave the
  height intact. Updates during sitting/flying wait until standing on the
  ground. On the grid: add/remove rigged shoes/body, rigid foot linksets and
  HUDs, including delayed mesh loading and attachments worn while seated.

- **`AURORA_DEMO_CAM`**: heading offset and pitch of the camera around the
  avatar (radians, a positive pitch looks down) and distance (metres).
- **`AURORA_DEMO_CAMERA=alt,x,y|pan,x,y|zoom,x,y|close,x,y[,n]|drift,x,y[,n]|tag|ml|wheel|fly|sit|scripted|scripted-sit`**:
  scenario logged under `demo camera` — Alt+click at (x, y) then drag and
  return while walking; `close`: Alt+click at (x, y) at frame 600, zoom for
  n frames (60 by default), then the camera no longer moves (close-up with a
  fixed camera, to compare consecutive screenshots); `drift`: the same
  close-up, then a very slow orbit (one fiftieth of a mouse pixel per frame)
  until frame 1200; press on our own name plate then drag (steering);
  entering / leaving mouselook (frame 650 shows "Press Esc to return to
  world view" above the bottom bar); wheel; pulling back in flight; sitting on a
  seat that rotates, with a seat camera and an animation started by a linked
  prim (frame 300), then standing up and a request to stop it (frame 700;
  `camera/demo.rs`).
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
- **`AURORA_DEMO_AGENT_JITTER=<mm>`**: our avatar receives five times a
  second (at 60 fps) a position correction of at most this size, like those
  a simulator sends to an avatar standing at altitude. With
  `AURORA_DEMO_CAMERA=close,x,y` on a rezzed object, the frame stays
  identical to the pixel; on our avatar, the camera follows it and all the
  nearby scenery jumps. Combine with `AURORA_DEMO_ALTITUDE` and
  `AURORA_STILL_CHECK=1`.
- **`AURORA_DEMO_KEY=down|up|left|right`**: holds an arrow from frame 235
  (movement, body orientation).
- **`AURORA_DEMO_SIT=n`**: clicks at frames 240, 300, 360…
- **`AURORA_DEMO_MMO="x,y[,1|2]"`**: left press on the avatar then right
  button held (steering with the mouse); `1`: right arrow instead,
  `2`: double right click (running).
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
- **`AURORA_DEMO_POINTER="x,y[,l|r|s][;x,y…]"`**: moves the interface
  pointer to these pixels of the window from frame 300, one point every
  60 frames (hover states, submenus); `,l` makes a left click, `,r` a right
  click on the interface at that spot. In the pie menu, the left click
  enters a submenu and the right click goes up one level. `,s` leaves the
  pointer still at that spot (it is only brought back there if it has moved
  away): this is what is needed to capture a tooltip, which egui only shows
  under a pointer at rest.
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
  **`AURORA_DEMO_LOOKAT=targets`**: `own`, and from frame 240 every other avatar looking
  somewhere new every 120 frames (focus and select points, the next avatar's
  eyes, free look, mouselook: one colour per type). With
  `AURORA_DEMO_CROWD=12`, `AURORA_DEMO_POS=129,134`,
  `AURORA_DEMO_CAMERA=close,1180,380,35`, `AURORA_FPS_LIMIT=60` and a capture
  at frame 860: crosshairs, names and lines among the crowd, hidden by the
  avatars in front of them.
- **`AURORA_DEMO_KEYBOARD=system|wasd|zqsd|qwertz|fallback`**: fresh default
  values of the movement keys and of the letter shortcuts with the Windows
  layout, a simulated US QWERTY / French AZERTY / German QWERTZ, or a failed
  detection (US QWERTY places); combine with `AURORA_DEMO_OPTIONS=8` to
  examine the shortcuts. With `zqsd`, Mouselook shows M, World map Ctrl+M
  and Snapshot to disk Ctrl+Ù (the AZERTY keys they are bound to); with
  `qwertz`, Snapshot to disk shows Ctrl+Ö. Saved bindings keep their keys,
  except the one-time move of the former US QWERTY defaults (settings
  version 12).
- **`AURORA_DEMO_CHATCMD="calc 2+2;rolld 2 20"`**: lines typed in the chat
  bar at frame 240, separated by `;` (chat bar commands: `calc`, `rolld`,
  `gtp`…).

## Objects and building

| Variable | Effect |
|---|---|
| `AURORA_DEMO_ACTIONS=<mode>` | Cursors, object clicks, buying and paying |
| `AURORA_DEMO_DIALOG=<mode>` | llDialog menu |
| `AURORA_DEMO_BUILD="<script>"` | Script of the build tools |
| `AURORA_DEMO_REPLACE_PRIM=1\|script` | Replacement board, script-only case |
| `AURORA_DEMO_BUILD_FRAME=<n>` | Start frame of the build script |
| `AURORA_DEMO_BUILD_FOCUS=off\|on` | Focus and camera following while editing |
| `AURORA_DEMO_BUILD_EDGE=<edge>` | Move to the edge, hold then release |
| `AURORA_DEMO_BUILD_TAB=<tab>` | Tab of the build window |
| `AURORA_DEMO_COLOR_PICKER=<mode>` | Colour picker and gestures offline |
| `AURORA_DEMO_SCRIPTS=<mode>` | Editing and saving scripts offline |
| `AURORA_DEMO_LSL_BRIDGE=1` | Messages of the Firestorm LSL bridge (hidden) |
| `AURORA_DEMO_FLEXIBLE=1` | Flexible prims: gravity, tension, wind, force and moving parent |
| `AURORA_DEMO_TEXREFRESH=1\|auto` | "Refresh textures" on a panel whose cached texture is stale |

### Details

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

`AURORA_DEMO_BUILD=create-delete,object:118` creates a primitive at
frame 625, selects it after the simulated reply and deletes it at frame 675.
Screenshot at 660: new cube selected; at 700: cube gone and message about
the move to the trash without a false owner warning. `create-ready` leaves
the cube to test its deletion by hand.

`AURORA_DEMO_BUILD=contents-copy,object:118` opens Contents and the
inventory, then copies an item at frame 645. `contents-move` offers the
confirmation of the removal of the non-copyable texture (with a warning for
scripts); `contents-move-plain` shows the case without scripts. Screenshot
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
Capture at 660 then 960: the window keeps its initial width of 530 points,
without growing on its own. The list keeps the folders, the names are
truncated and their tooltip shows the whole text. The splitter adjusts the
room given to the names, the preview stays bounded and the controls are
grouped at the bottom. The collapsible "Local files" folder comes before the
inventory in the diffuse texture and PBR material pickers. It lists the
local previews already loaded, textures or materials depending on the choice
requested, and follows the search filter. A click applies the local preview;
"OK" keeps it, "Cancel" restores it. To show the synthetic local files,
combine this scenario with `AURORA_DEMO_IMPORTS=local`.
`faces-picker-material,object:99620` opens the same picker for a PBR
material: only the local material must be listed. These files appear without
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

**`AURORA_DEMO_SCRIPTS=inventory|object|both|save|object-save|error|readonly|syntax|object-syntax|syntax-readonly|indent|object-indent|tabstops|object-tabstops|close-brace|object-close-brace|colors|color-picker|color-accept|color-cancel|comment-lines|comment-block|multiline-indent|multiline-outdent|folding|object-folding|experience|experience-save|workbench|object-workbench|insert-filter|insert-empty|search|preferences|help|preprocessor|object-preprocessor|processed|preprocessor-error`**:
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
`preprocessor` and `object-preprocessor` save a script using an include from
the inventory, a macro, an extended list and a `switch`, then reopen it: the
original text must be kept. `processed` shows the result sent to the
simulator, read-only; `preprocessor-error` shows an error before sending,
with the draft kept.
New `llOwnerSay` messages open Script debug if their routing is enabled; the
errors of owned objects also appear there, after filtering of the bridge and
of blocked objects. The window allows seeing all the objects or a single one
and clearing the displayed messages.

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
- `play` starts a local HTML page; `pause` and `open-media-playing`
  inject a simulated playback state before the click; `open-media` simulates
  the opening of the external browser.
- `zoom` frames the cube.
- `tp-door`: the Touch cube answers like a club's teleport door scripted with
  `llMapDestination` (ScriptTeleportRequest): the world map opens centred on
  a tracked spot of the plaza region, at 118 m; nothing teleports until the
  map's Teleport button. `tp-door-ready` stops before the click.
- `buy-confirm`, `pay-confirm`: confirm the simulated transaction.
- `linked-touch`, `linked-sit`: hollow root with Touch and child seat with
  Sit, overlapping bounding boxes; checks the prim actually aimed at.
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
  Left click must reach panel 976 and leave `dialog=false`. The GPU response
  `demo drawn pick` must include it in `blend` for the first three modes,
  even at zero alpha. `mask-zero` uses the zero-alpha texture in Mask mode:
  the click must reach Buy (971), with `dialog=true` and no panel in the GPU
  response. Capture after frame 620.
- Add `-ready` to the five Blend / Mask modes above to test hover without
  an automatic click. At frame 650, `demo cursor hover` must report
  `prim=Some(976), action=None` before Blend at every opacity, and
  `prim=Some(971), action=Some(Buy)` through the Mask hole. Enable
  `AURORA_HOVER_CHECK=1` to compare the cache and index with the full search;
  media hover behind Blend must respect the same depth limit.

Transaction windows, with the same variable:

- `pay-layout`, `pay-hidden`, `pay-large`: four four-digit amounts, free
  field and hidden buttons, maximum amount.
- `buy-original`, `buy` (copy), `buy-contents`: purchase of an original, of
  a copy or of the contents, list of the included items and permissions of
  the next owner.
- `buy-empty`: offers contents that cannot be sold and disables the purchase.

**`AURORA_DEMO_DIALOG=12|4|long`** — llDialog menu of twelve answers in
three columns, arranged from the bottom to the top like Firestorm; `4`: four
answers, to check the incomplete last row; `long`: long label truncated,
with the full text on hover. Block and Ignore buttons under the menu.

**`AURORA_DEMO_BUILD="mode,x,y[,part,dx,dy]"`** — script of the build
tools. Modes: `move`, `rotate`, `stretch`, `face`, `align`, `grab`,
`focus`, `create`, `land`, `select`. `AURORA_DEMO_BUILD_FRAME` changes its
start frame.
The variant `mode,object:<local id>[,part,dx,dy]` selects an object of the
demo without fixed click coordinates.

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

**`AURORA_DEMO_BUILD_TAB=general|object|transform|shape|features|texture[:pbr|bp|media]|contents`**
— tab of the build window shown by the script; `+weights`, `+grid`, `+media`
also open these windows.

**`AURORA_DEMO_LSL_BRIDGE=1`** — messages of the Firestorm LSL bridge
(hidden) between two "owner say" lines.

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


## HUDs

| Variable | Effect |
|---|---|
| `AURORA_DEMO_HUDS=<mode>` | Worn HUDs: display, touch, move, edit |
| `AURORA_DEMO_WORN=<mode>` | Object worn on the body: menus, Edit, Inspect |

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
- `drag-zoom`: the same at 50%; `drag-ignore`: on a child with the
  IGNORE action; `drag-ctrl`: explicitly binds Ctrl; `drag-key`: the B key
  instead of Ctrl; `drag-secondary`: ALT as primary key and Ctrl as
  secondary key, moving with the latter.
- `menu`, `edit`, `edit-zoom`: right click on the child, Edit and
  moving the linkset by the Y handle, also at 50%.
- `edit-scroll`: selects a HUD locally enlarged to 200%, then
  places it off screen (frame 740). Without the HUD key, five notches
  backwards in an empty area zoom all the HUDs out (frame 760); with the HUD
  key, one notch enlarges only the hovered HUD (frame 820). Five notches
  forwards return to the normal global view (frame 900). The camera and the
  local factors are kept during the global zoom; the HUD key keeps
  its priority for the local zoom. Frames 920 and 940 close editing,
  then select a world object: the wheel is no longer intercepted
  by the HUDs. Suggested screenshots: 750,790,850,910.
- `faces`, `faces-turned`: nine HUD panels with a single visible face —
  columns front / back / explicitly double-sided PBR material, rows
  opaque / translucent / masked. The back column must stay empty;
  `faces-turned` turns all the panels by 180°: the front column
  becomes empty, the back appears, the double-sided one stays visible. The
  floating labels stay visible both ways.

**`AURORA_DEMO_WORN=1|menu|edit|move|inspect`** — our avatar wears on its
back (Spine point) a two-prim object, touchable, modifiable, linked to an
inventory item ("Demo backpack", in the current outfit). No
access to a grid.

- `1`: worn object only, to test by hand the right click on the object,
  in the inventory and in Appearance › Current outfit.
- `menu`: right click on the object at frame 700 (`menu_attachment_self.xml`).
- `edit`: then "Edit" at frame 740: build window, object
  outlined, arrows aligned on the attachment point, local position in
  Transform.
- `move`: like `edit`, then drags the Z arrow by 80 px (frames 780–830); the
  log writes `demo worn edit: local position=…, attachment=9`.
- `inspect`: "Inspect" at frame 740: the window lists the root and the
  linked prim, their figures and the totals; the prim of the highlighted
  row is outlined in yellow, the others in blue.

The steps are counted in frames: capture with `AURORA_FPS_LIMIT=60`,
around frames 720 (menu), 770 (edit, inspect) and 850 (move).

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
| `AURORA_DEMO_ANIM_LOOP=1` | Animation loops, sequences and stops |
| `AURORA_DEMO_AO=1` | Built-in AO and synthetic animation inventory |
| `AURORA_DEMO_ANIMESH=1` | Animated objects (animesh) |
| `AURORA_DEMO_ANIMESH_TRIS=<n>` | Triangles added to the demo's rigged mesh |
| `AURORA_DEMO_CLOUD=1` | Avatar loading clouds |
| `AURORA_DEMO_CLASSIC=1` | Classic avatars: shapes and bakes with alpha |
| `AURORA_DEMO_DEFORM=1`, `reset`, `reset-anim`, `self` or `self-reset` | An avatar deformed by a finished pose, and "Reset skeleton" |
| `AURORA_DEMO_DISPLAYNAME="<name>"` | Display name change (simulated) |
| `AURORA_DEMO_DISPLAYNAME_ERROR=<error_tag>` | Server error on the name change |

### Details

- **`AURORA_DEMO_ANIM_LOOP=1`**: loop whose first interval is missing,
  sequence changed every 120 frames, stop / restart at frames
  960 / 1020 every 1200 frames.
- **`AURORA_DEMO_AO=1`**: opens the AO manager, with two standing poses in
  a cycle, walk, run, sit, fly, typing and an importable notecard.
  The movement keys change the state. `transitions` chains the
  states automatically after loading, every 180 frames;
  `off` starts with the AO disabled. Check the AO button at the bottom, the
  poses, the pause, the cycles and adding / importing from the inventory.
- **`AURORA_DEMO_ANIMESH=1|churn`**: animated root mesh, linked mesh flagged
  by a child and worn animesh, with independent skeletons of their own and
  alpha shadows. `churn`: the animesh on the ground (orange) also changes its
  mesh asset every 240 frames, like the rigged meshes that come and go on
  the grid; it must stay whole and animated at each change, the other
  two as well, and the line `demo animesh: skin bindings <in use> used
  of <in the list>` of the log (every 120 frames) must stop growing
  once the first meshes are freed (600 frames after their last use).
- **`AURORA_DEMO_ANIMESH_TRIS=<n>`**: with `AURORA_DEMO_ANIMESH`, adds
  about n triangles to the demo's rigged mesh, as opaque faces of 45,000
  (each a thin tube around the torso, 7 faces at most). With
  `AURORA_DEMO_CROWD=<n>`, each avatar of the crowd wears this mesh: 200000
  gives avatars as heavy as those of the grid (3.6 million
  triangles on screen for 16 avatars drawn in full), which is what is needed
  to measure the passes that redraw the avatars (shadows, prepass).
- **`AURORA_DEMO_CLOUD=1|self`**: `1` enables offline the clouds and the
  loading bars. `self` also dresses our avatar like those that
  stayed a cloud for the 60 s of the limit (issue #121): invisible head
  bake (alpha layer), textured upper and lower body, a worn mesh that the
  asset server does not have (404) and a hat. It must appear about
  3 s after its arrival, without a head and without the mesh, and the log
  must contain "avatar … (own) shown after 3.0 s". Screenshots with
  `AURORA_FPS_LIMIT=60` and `AURORA_CAPTURE_FRAMES=240,1200`: cloud, then
  avatar.
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

## Rendering and environment

| Variable | Effect |
|---|---|
| `AURORA_DEMO_PARTICLES=<mode>` | Wind and smoke particles, or particle load |
| `AURORA_DEMO_PLANAR=1` | Slabs with planar texture mapping |
| `AURORA_DEMO_PBR_OVERRIDE=1` | GLTF material override |
| `AURORA_DEMO_TEXANIM=1` | Texture animations (llSetTextureAnim) |
| `AURORA_DEMO_GLOW_LAYERS=1` or `shell` | Transparent glowing layer over a glowing skin |
| `AURORA_DEMO_PROJECTOR=1` or `point` | Projectors (lights with a projected texture) |
| `AURORA_DEMO_SKY=<gamma>[,<haze>[,<ambient>]]` | Classic EEP sky with this gamma |
| `AURORA_DEMO_SKY_OPTICS=rainbow\|halo\|clear` | Rainbow or 22° halo in the sky |
| `AURORA_DEMO_OCCLUSION=1` | Wall with hidden objects (occlusion test) |
| `AURORA_DEMO_DEBUG="<overlays>"` | Debug overlays |
| `AURORA_DEMO_EEP_PARCEL=<mode>` | EEP environment of the region and the parcel |
| `AURORA_DEMO_ENV_SELECT=<mode>` | Environment picker |
| `AURORA_DEMO_ENV_EDITOR=<mode>` | Sky, water and day cycle editors |
| `AURORA_DEMO_DOF=<f>:<m>[:debug]` | Depth of field at this aperture and focus distance |

### Details

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
  `load`: instead, 32 more fountains in a circle around the plaza,
  that is about 4,000 live particles, to measure the `p_part_*` parts
  of the profile (combine with `AURORA_DEMO_STREAM=<n>` for the
  number of objects).
- **`AURORA_DEMO_PLANAR=1`**: the tiles must line up from one slab
  to the next.
- **`AURORA_DEMO_PBR_OVERRIDE=1`**: two PBR slabs, one of them with a
  GLTF material override (4 × 4 repeats, tint). Their normal map,
  also carried by the "Normal + specular" panel of the ordinary
  demo, contains only bumps: each must be light on the side
  of the sun (at the top of the panel with `AURORA_DEMO_TOD=2`). Dark at the
  top and light at the bottom, the relief is inverted.
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
- **`AURORA_DEMO_SKY=<gamma>[,<haze>[,<ambient>]]`**: classic EEP sky (without
  reflection probe ambiance) with this sky gamma and a sun colour
  above 1: legacy gamma and object light normalised like
  Firestorm. `haze`: haze distance multiplier (0.8 by
  default; `37` gives a hazy sky like many regions have,
  where what is a few dozen metres away is veiled). With
  `AURORA_DEMO_CROWD=40` and `AURORA_MAX_AVATARS=4`, the impostors must
  be veiled like the avatars drawn in full next to them
  (`AURORA_MAX_AVATARS=60` to compare). `ambient`: ambient light of the
  sky (0.25 by default, more than 1 in the brightest skies); with
  `AURORA_DEMO_SKY=1.62,0.8,1.1` and `AURORA_DEMO_PBR_OVERRIDE=1`, the surround
  of each bump of the slabs must be darkened by ambient occlusion, and no
  longer be once ambient occlusion is turned off in the settings.
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
- **`AURORA_DEMO_DEBUG`**: comma-separated list among `bounds`,
  `culling`, `lights`, `probes`, `skeletons`, `alpha`, `wire`, `complexity`,
  `glow`, `glow_view`, `freeze`, `lod`, `shadows`, `ao`, `check_lights`,
  `check_probes`: the entries of Preferences › Debug. `shadows` is the view
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
- **`AURORA_DEMO_ENV_EDITOR=sky|clouds|sun|beacons|water|item|item-save|day|day-play|day-edit|day-notrans|parcel|region|custom|custom-follow`**: sky,
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
| `AURORA_DEMO_OVERDRAW=<n>` | Overdraw test: n rows of opaque walls |
| `AURORA_DEMO_STREAM=<n>[,<wave>][,leave][,j2k=<folder>][,net=<ms>][,size=<m>]` | Streaming stutter test; with `net=`, textures downloaded from a local server |
| `AURORA_DEMO_HAIR=<layers>[,<segments>[,<empty>[,<period>]]]` | Transparent face load: stacked shells on the avatars |
| `AURORA_DEMO_LIGHTS=<n>` | Local light load: n small lights in the crowd |
| `AURORA_DEMO_PROBES=1` | Two manual reflection probes on the plaza: a box and a sphere |
| `AURORA_DEMO_GEOMETRY=<mb>[,<mb per second>]` | Geometry load: meshes filling the geometry arena |

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
  - To read in `perf summary`: `g_transparent` + `g_post` (see
    [Profiling](#profiling)), `blend`, `blend_tris_k`. `32` with
    `AURORA_DEMO_CROWD=40` and `AURORA_DEMO_POS="129,134"` gives ~970
    transparent faces (17 avatars drawn in full).
  - For the early discard of invisible fragments
    (`AURORA_BLEND_EARLY_DISCARD`): it only pays off where empty
    areas are in one piece on screen. `32,24,90,256` (large empty
    stretches) shows it, `32,24,60,16` (fine strands) gains nothing.
- **`AURORA_DEMO_GEOMETRY=<mb>[,<mb per second>]`**: after the loading fade,
  meshes of 60,000 vertices keep entering the geometry arena until it
  holds `mb` megabytes of them (800 if the value is not a number), at 25 MB
  a second unless told otherwise (Knotty beach loads about 22 MB a second
  for 35 s). They are not drawn: the scenario only makes the arena outgrow
  its buffers, in the frames of the ordinary demo. The log gives each
  growth (`vertex arena grown to … MB`) and the end (`demo geometry: …
  meshes, … MB in the arena after … s`). With `AURORA_PROFILE=1`, read
  `max=` and `r_acquire` in `max:` on the seconds of a growth; with
  `AURORA_PROFILE_FRAMES=1`, the `render profile` line of the slow frame.
  What a growth costs is the first use of the new buffer, about 21 µs per
  MB (`aurora-render/src/arena.rs`).
- **`AURORA_DEMO_LIGHTS=<n>`**: n small local lights (64 if the value
  is not a number, 256 at most), glowing spheres of 6 cm at
  head height, spaced 1.6 m apart on the crowd's plaza and beyond,
  with a 3 m radius (4.5 m once extended like Firestorm's deferred
  lights): the "face lights" of a crowd and the lamps of a club. The
  renderer takes the 64 nearest (`lights=` in `perf summary`). With
  `AURORA_DEMO_POS="129,134"` they are in front of the camera, 9 m away and
  more; with `"140,134"`, the camera is in the middle of them. Used to measure
  `AURORA_LIGHT_TILES`.
- **`AURORA_DEMO_PROBES=1`**: two manual reflection probes, as invisible
  prims on the plaza: a box of 10 × 8 × 6 m turned on itself, which the
  camera of `AURORA_DEMO_POS="135,134"` stands in, and a sphere of 12 m east
  of it. The automatic probes are kept out of the box, and mixed with the
  sphere's by its weight. Used to check `AURORA_PROBE_WALK` on the probes
  the plaza does not have otherwise (its only box probe is the mirror's,
  8 cm thick).
- **`AURORA_DEMO_OVERDRAW=<n>`**: n rows of opaque walls (12 if the
  value is not a number, 40 at most) east of the plaza, one
  behind the other and each taller than the one in front, sent from the
  farthest to the nearest. Each wall sticks out above the next: occlusion
  keeps them all, and a scene pass that starts from an empty depth
  computes the colour of each in full before the nearer ones cover
  it. This is the worst case of overdraw, to measure
  what a prepass saves (`AURORA_MSAA_PREPASS`); a real region has
  less.
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

## Interface

| Variable | Effect |
|---|---|
| `AURORA_DEMO_TOOLBAR=1` | Open the "Toolbar buttons" window; the bars mockup's custom state, side toolbars, labels. |

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
without the window), `right` (them in the right toolbar), `labels` (the
dock in icons and labels, the left toolbar in labels). Capture at frame 780.

The values of these switches are English words. The French words they had
before (`amis`, `groupes`, `repere`, `historique`…) are still accepted.

| Variable | Effect |
|---|---|
| `AURORA_DEMO_WINDOWS=<mode>` | Collapse, move and restore of the inventory title bar |
| `AURORA_DEMO_MINIMIZED=<mode>` | Minimized Conversations: short bar, unread outline, expanded by its toolbar button |
| `AURORA_DEMO_SNAP=<mode>` | Window snapping: Alt attachment, free move, detachment, grouped move, resize |
| `AURORA_DEMO_RESIZE=<mode>[,<window>]` | Resize marks of a window: corner or edge hovered, corner dragged |
| `AURORA_DEMO_EXIT=<mode>` | Exit modal and offline choices |
| `AURORA_DEMO_UI=<tab>` | Opens People (tab), the inventory and the chat |
| `AURORA_DEMO_RADAR=1`, `tp`, `friends`, `groups` or `profile` | People window: radar, radar after a teleport, Friends tab, Groups tab, a group's profile |
| `AURORA_DEMO_PEOPLE=rows`, `hover`, `crowd`, `crowd-hover`, `sheet`, `legacy`, `friends`, `groups`, `sets`, `recent` or `blocked` | People with the mock-up's contents: Near you (rich rows, compact crowd, a person's sheet, Firestorm's table) and the other tabs |
| `AURORA_DEMO_OPTIONS=<section>` | Opens the preferences on a section |
| `AURORA_DEMO_PERF=compact`, `full` or `network` | Opens the Performance window: compact, full, or as the bars open it on its Network section (full for this opening, the section unfolded and scrolled to) |
| `AURORA_DEMO_BUY_CURRENCY=<mode>` | Demo balance and "Buy L$" window |
| `AURORA_DEMO_BUY_LAND=<mode>[:confirm]` | Plaza parcel for sale and "Buy Land" window |
| `AURORA_DEMO_PAY=<mode>` | Demo balance and the Pay window of a resident or a group |
| `AURORA_DEMO_LOGIN=<mode>` | Offline login screen and account carousel |
| `AURORA_DEMO_LOGIN_SUBMIT=<frame>` | Press Log in of the open login form at that frame |
| `AURORA_NEWS_URL=<url>` | Login news read from somewhere other than `main` |
| `AURORA_DEMO_CRASH=<mode>` | Offline crash and recovery window |
| `AURORA_DEMO_CRASH_DIR=<folder>` | Isolates the crash reports of offline tests |
| `AURORA_CRASH_CAPTURE=<file.png>` | Screenshot of the recovery window in demo |
| `AURORA_CRASH_CAPTURE_EXIT=1` | Closes the window after its screenshot in demo |
| `AURORA_DEMO_UPDATE=<mode>` | Interactive update mock-up on the login screen |
| `AURORA_DEMO_UPDATE_MAINTENANCE=<mode>` | Announces a cache invalidation and/or a reset |
| `AURORA_UPDATE_TEST_ROOT=<folder>` | Throwaway installation served by a local feed |
| `AURORA_UPDATE_TEST_VERSION=<tag>` | Dev build with a synthetic version for this test |
| `AURORA_DEMO_BUG_REPORT=<mode>` | Public bug report with demo data, nothing sent |
| `AURORA_DEMO_REPORT_ABUSE=<mode>` | Report Abuse window, its checks and its sending, nothing sent |
| `AURORA_DEMO_MAP=1`, `mini` or words among `search:<text>`, `drawer:<tab>`, `landmarks`, `friend`, `legacy`, `coordinates`, `beacon[-click|-offscreen]` | Maps, search, drawer, landmarks, followed friend, side panel layout, coordinate entry and clickable 3D beacon |
| `AURORA_DEMO_NOTIF=1`, `AURORA_DEMO_STATUSMENU=1` or `invisible`, `AURORA_DEMO_MICMENU=1`, `AURORA_DEMO_NAVEDIT=1` | Notification list, status menu, microphone panel, location field |
| `AURORA_DEMO_BARS=<words>` | Top and bottom bars: islands and dock, Firestorm's bars, place island stretched or not, rights named, talking, wide chat island, the favourites (menu, row, dock), a menu open, the interface hidden |
| `AURORA_DEMO_TOASTS=1\|click\|minimized` | Friend online/offline and IM toasts, unread pill, click on an IM card |
| `AURORA_DEMO_NOTIFS=1\|dnd\|list\|history\|legacy\|legacy-history` | Notification center: cards unrolling from the bell, burst, paused bell, list and history, Firestorm toasts |
| `AURORA_DEMO_PRESENCE=<place>` | Friends' online / offline toasts at one of their six places, waiting under the bell's cards and list |
| `AURORA_DEMO_RESTART=<seconds>[:manager][:fast][:still][:min][:lm]…` | The demo region announces its restart: the alert card, its steps, pill, bell card, Firestorm's box |
| `AURORA_DEMO_COLORPICK=1` | Opens the compact colour picker of the first swatch shown |
| `AURORA_DEMO_CONV=1`, `AURORA_DEMO_TALK=1` | Group conversation, open microphone |
| `AURORA_DEMO_CONV=bubbles[:busy\|group\|local\|legacy]` | Conversations as a messenger: bubbles, head with picture and status; or the legacy lines |
| `AURORA_DEMO_LURE=<case>` | Teleport offers and requests received |
| `AURORA_DEMO_MATURITY=agree\|disagree\|fail` | Content rating changed, the server's answer |
| `AURORA_DEMO_SLURLS=1\|offer` | Place links with their region rating badges, one region looked up |
| `AURORA_DEMO_EXPERIENCE=1\|auto\|profile\|blocked\|walk\|arrive` | Experiences asking: a door to click, a pad to walk into, a parcel greeter on arrival |
| `AURORA_DEMO_CONTACTS=<tab>` | Contacts tab of Conversations |
| `AURORA_DEMO_CHATLOG=<mode>` | Conversation history: files, search, History window, offline IMs |
| `AURORA_DEMO_GLYPHS=1` | Names, titles and chat in extended characters (system fonts) |
| `AURORA_DEMO_EMOJI_PICKER=1\|<text>` | Conversations open on the emoji picker, a text in the input field |
| `AURORA_DEMO_PROFILE=<avatar>[:tab\|edit\|compact\|form\|publish]` | An avatar's profile window |
| `AURORA_DEMO_FEED=<url>` | Profile "Feed" tab on this page |
| `AURORA_DEMO_APPEARANCE=<mode>` | Appearance window and outfits |
| `AURORA_DEMO_SHAPE=<mode>` | Worn shape editor and permission scenarios |
| `AURORA_DEMO_INVENTORY=<mode>` | Synthetic inventory, menus and windows |
| `AURORA_DEMO_GESTURES=manager\|editor\|play\|completion\|completion-send\|communicate\|communicate-open` | Gesture manager, editor, playback, completion and menu access |
| `AURORA_DEMO_IMPORTS=<mode>` | Import window and synthetic assets; `launcher` / `settings` open the compact chooser / import defaults |
| `AURORA_DEMO_IMPORT_FILE=<path>` | Local asset to open in the demo flow |
| `AURORA_DEMO_IMPORT_REGISTRY=<file>` | Isolated persistent library for the import tests |
| `AURORA_DEMO_REZ=<mode>` | Rez by drag and drop from the inventory |
| `AURORA_DEMO_LAND=<tab>[:role][:classic]` | "About Land" on a page or a tab, as owner, officer, member or visitor |
| `AURORA_DEMO_LAND_PRIMS=<used>[,<step>]` | Objects of the demo parcel, which increase with each answer |
| `AURORA_DEMO_REGION=overview\|manage[:role][:preview][:compact][:section][:pending][:ask-…]` | "Region / Estate" as estate owner, manager or resident |
| `AURORA_DEMO_GROUP_PROFILE=<scenario>[:members\|:invite\|:notices\|:compose\|:admin\|:identity\|:land\|:experiences]` | Group profile window, as a member, officer, owner or guest |
| `AURORA_DEMO_GROUP_CREATE=form\|cost\|refused\|limit\|created[:classic]` | "Create a group" window and its simulated server |
| `AURORA_DEMO_PLACE=<place>` | "Places" on a place profile |
| `AURORA_DEMO_PLACES=<tab>` | "Places" on a tab |
| `AURORA_DEMO_LANDMARK=<mode>` | "Create landmark" window, or the lit landmark star |
| `AURORA_DEMO_PLACE_WINDOW=1` | Place profiles in separate windows |
| `AURORA_DEMO_TP_CONFIRM=<origin>` | Teleport confirmation on screen |
| `AURORA_DEMO_SEARCH=<tab>[:text]` | "Search" on a tab, search done |
| `AURORA_DEMO_SEARCH_WEB=<page>` | Page loaded by the Web tab of "Search" in demo |
| `AURORA_DEMO_MARKETPLACE=<page>` | Marketplace window open at start on this page |
| `AURORA_DEMO_MARKETPLACE_LISTINGS=<mode>` | Marketplace listings window open on a simulated SLM server |
| `AURORA_DEMO_BAN=1` | Ban lines |
| `AURORA_DEMO_RESTRICTED=1` | Parcel that forbids everything |

### Details

- **`AURORA_DEMO_PAY=<mode>`**: gives a balance of L$ 1,000 and opens the
  Pay window, Tess as the payee, in the look of Preferences › Aurora › Pay
  window. A payment only moves the demo balance (no L$, no request sent).
  `resident`: L$ 25 typed, Pay sends it and the window says it was sent.
  `short`: L$ 1,500, the band with Buy L$ and Pay greyed out. `confirm`:
  L$ 500, the confirmation asked above the threshold (Preferences ›
  Payments). `sent`: the end of a payment, with its message. `group`: the
  demo group as the payee, without a message field. An object's payment
  is `AURORA_DEMO_ACTIONS=pay`.
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
- **`AURORA_DEMO_IMPORTS=launcher|settings`** opens the compact horizontal
  import chooser or its saved-defaults window, without loading fixtures or
  opening a disk dialog. The configurable Import shortcut (Ctrl+U by default)
  toggles the chooser; a type button opens the filtered picker when its list
  is empty, otherwise the existing inspector. Clicking the displayed type
  closes its inspector. The icon bar stays open. Check that another type can be clicked while
  an inspector is displayed, even when a narrow viewport causes overlap.
  Build › Import performs the same toggle; Contents has no Import entry.
  The glass variant has eight independent frosted buttons with clear gaps,
  no group background, and a plain close cross at the right.
  The Settings icon toggles the permissions/payment window, closing it on the
  second click even if its title bar was minimized.
- **`AURORA_DEMO_IMPORTS=inspectors`** opens the chooser with fixtures already
  loaded for every type, including local previews. A type button therefore
  opens its inspector directly without a disk dialog, as in the normal viewer.
  Check each dedicated window and its filtered file list, then Multiple's
  complete tabs. Permissions belong only to Settings. First-opened windows
  have the same top-left position; moved windows retain their placement.
- **`AURORA_DEMO_IMPORTS=texture|destination|material|material-cube|material-aurora|material-maps|material-loading|material-zoom|material-texture-zoom|mesh|mesh-normals|mesh-zoom|mesh-low|mesh-error|mesh-physics|mesh-degenerate|mesh-options|permissions|animation|animation-own|sound|bulk|bulk-mesh-denied|bulk-upload|local|local-mesh|local-mesh-applied|local-mesh-removed`**:
  writes files specific to this launch into `.scratch/import-demo/<uuid>/`
  then opens the chosen flow: RGBA image, PBR material, Collada cube,
  BVH / `.anim` animations and synthetic WAV sound. Check the settings, the
  previews, the camera, the permissions, the selection and the confirmation.
  Each type has a dedicated window with the file list at the left and only
  its relevant metadata, preview and controls at the right. There
  are no other type tabs in these windows. Sound uses a shorter window.
  `bulk` is labelled Multiple and retains every type tab and the existing
  mixed import flow. Switching its tabs keeps the
  same window and never reopens the picker.
  `material-cube` / `material-aurora` select the additional preview shapes.
  `material-maps` shows four unframed rows in one column below a compact preview,
  95.2-point square thumbnails (1.7× the previous size) preserving original proportions, left-aligned titles
  and texture names, right-aligned commands and common parameter rows.
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
  labelled and easing fields aligned. The batch includes a mesh and needs its quote
  before confirmation. Ctrl+U toggles the compact chooser; the file picker
  opens after clicking a type. Rebind it in Preferences › Shortcuts.
  The quote and the uploads stay entirely inside the demo server.
  `destination` opens the folder choice with the "Textures" filter;
  typing another name must filter without changing the destination or
  closing the list. Choosing a result closes the list and applies that folder.
  Each type has its own default size, with narrower file lists for Material
  and Animation and closer checkbox / name spacing. Name, Description and
  Destination use the same three rows above every asset panel, including
  meshes; Upload no longer repeats them. Preview and LOD cards are side
  by side, with gesture hints directly under the preview and compact tools.
  High is shown by default; clicking anywhere within a LOD card selects
  its preview, including its fields and counters. File / Browse share
  a row, generated criteria / targets are below; results and retained
  percentage are right-aligned, and triangles are spelled out.
  `mesh-normals` shows 105° without a stray character or clipping.
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
  `local` loads texture, material and mesh together through the same filter.
  `AURORA_DEMO_IMPORT_REGISTRY=<test file>` enables the persistence of the
  import scenarios to this isolated file. First launch `local-texture-applied`,
  `local-material-applied` or `local-mesh-applied`, then `local-restored` in
  a new process with the same file and `AURORA_DEMO_ACTIONS=1`.
  The second launch creates no new asset: only the library and
  the saved links must restore the GPU application. Without this
  variable, the synthetic scenarios do not fill the persistent library
  of the demo profile. The synthetic sources stay in `.scratch`.
  `animation-own` puts the preview at 0.75 s, paused, also on the demo
  avatar. The slider and the pose must stay fixed until Resume.
  `AURORA_DEMO_IMPORT_FILE` replaces the fixtures with an existing file,
  read without modification; choose the mode matching its type.

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
- **`AURORA_DEMO_WINDOWS=1|restore|drag`**, with `AURORA_DEMO_INVENTORY=1`:
  double-click on the real title bar of the inventory at frames 660–665; `1`
  keeps the bar collapsed, `restore` expands it at frames 1000–1005,
  `drag` also moves it by 100 × 60 points at frames 780–782.
  Take screenshots at `620,740,900,1120` to check content, collapse, move and
  restored dimensions. The "−" button stays usable; the other
  title bar buttons and the content do not trigger the title bar double-click.
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
- **`AURORA_DEMO_RESIZE=corner|edge|drag[,<window>]`**: from frame 600,
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
  `AURORA_FPS_LIMIT=60` at frame 780.
- **`AURORA_DEMO_NOTIF=1`**: the "Aurora lantern" offer is from a
  resident: the object is already in the inventory (Objects folder) when
  the offer arrives. "Keep", or closing the offer in the list, leaves it
  there; "Reject" sends it to the Trash. With `AURORA_DEMO_INVENTORY=offer`,
  the inventory opens on the received object and the offer is still to be
  answered; `offer-reject` rejects it at frame 700, then shows the object in
  the Trash. Take the screenshot after frame 900.
- **`AURORA_DEMO_NOTIFS=1|dnd|list|history|legacy|legacy-history`**: the notification
  center's mockup, at 60 frames a second. An IM from Nova at frame 700
  unrolls from the bell at the right end of the top bar (violet bar of the
  time left, 6 s for information, 12 s for what waits for an answer), folds
  back into it around frame 1180 and the red count bounces; Orion comes
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
- **`AURORA_DEMO_TOASTS=1|click|minimized`**: a friend goes offline at
  frame 700 and comes back at 760 (two presence toasts, round picture with
  its status dot), two IMs from the same resident arrive at 820 and 880 (one
  card, "(2)"), a group IM at 940 (group toasts are forced on for the
  scenario). The Conversations button of the bottom bar shows the unread
  IMs and group chats in the red counter (5 at frame 1000), not the nearby
  chat lines.
  The friends announced at login get no toast, only their chat line. `1` opens the bell list at frame 1100: only the
  two conversations are listed, never the presence notices. `click` (window
  closed) and `minimized` (window minimized at start) click the first IM
  card at frame 1250: the Conversations window opens or expands, comes to
  the front on that conversation, and its card leaves the list. Capture
  with `AURORA_FPS_LIMIT=60`, at `900,1160` for `1` and `1240,1310` for the
  click modes. Add `AURORA_DEMO_SOUND=1` to hear the existing sounds.
- **`AURORA_DEMO_EXIT=1|cancel|logout|quit`**: opens the modal at frame 650.
  `1` leaves it interactive; the other modes make the choice at frame 800.
  Take screenshots at `740,900` to compare the blurred background then the
  resumed session or the return to the login. `quit` ends the process at
  frame 800 (capture only 740, without `AURORA_CAPTURE_EXIT`). Combine with
  `AURORA_DEMO_LOGIN=empty` for the login screen: Log out is
  disabled. The choices are simulated offline, with no login request.
- **`AURORA_DEMO_OPTIONS=<section>`**: for example `8` for Shortcuts
  (HUD move key). `8:<query>` opens Shortcuts with its search filled in,
  for example `8:Ctrl` for assigned keys or `8:HUD` for a category;
  `8:zzzz` shows the empty result. The field stays above the scrolling list,
  and its clear button restores all actions. Search follows the selected language.
  `13` opens the Aurora page: the aurora entry at the top of the list, the
  page head with "All in Firestorm version" / "All in Aurora version", then
  the Appearance, Gaze and Avatar plates groups; each choice changes its
  element at once (conversations, name tags, look-at targets with
  `AURORA_DEMO_LOOKAT=own`, gaze under the cursor). `<section>:end` (other
  than `8`) opens the page scrolled to its end, for example `13:end` for the
  Avatar plates group. `<section>:reset` (other than `8`) opens the "Reset
  this section" confirmation of that page, for example `0:reset` ("Reset
  Graphics?"). `0` shows the Presets row at the top of the Graphics page:
  with `AURORA_GRAPHICS_PRESET` its level is lit, otherwise (the defaults)
  no segment is and "Custom" follows the row.
  With `AURORA_DEMO_LOGIN` (for example `empty` or `cards`) the preferences
  open over the login screen, as its top right cogwheel does: every page
  works before login, and Chat › Conversation logs says that the folder
  belongs to an account.
- **`AURORA_DEMO_COLORPICK=1`**: at frame 300, opens the compact colour
  picker (`widgets::color_button`) of the first colour swatch
  drawn. To be combined with `AURORA_DEMO_OPTIONS=5` (Colors): the
  panel opens under "My name", with the saturation / value field, the
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
  except Cyrillic, Greek, emojis and a few symbols). Visible limits: Arabic and
  Hebrew words are well formed, but those of a sentence follow one another
  from left to right; flags, skin tones and ZWJ sequences not composed; emojis
  monochrome outside the chat. On a system without the expected fonts
  (see `ui/fonts/fallback.rs`, verified on Windows 11), the squares
  remain for the missing scripts; the log lists the fonts
  found (`system fallback fonts: …`).
- **`AURORA_DEMO_EMOJI_PICKER=1|<text>`**: the Conversations window open on
  local chat with its emoji picker shown (first tab, smileys); any value
  other than `1` is put in the input field, as if picked. With
  `AURORA_DEMO_GLYPHS=1`, the emoji line of the chat is visible beside it;
  with `AURORA_DEMO_CHATCMD=<same text>`, the line is sent.
  What to see: colour emojis in every cell, centred, with their
  proportions (the 2D font's images are wider than tall), and the same
  colour emojis, same size, in the input field and in the sent line.
  `AURORA_EMOJI_FONT=<file>` compares two fonts.
- **`AURORA_DEMO_CRASH=panic|native|missing|normal|recovery`**, only
  with `AURORA_DEMO`: triggers a Rust panic, a native Windows
  exception, an abnormal exit without a trace, a normal close or an
  exit without a watcher. The last mode makes it possible to check the
  recovery at the next launch (relaunch with `normal`). No
  connection and no GPU initialisation. The window must keep the language
  and the theme of the stopped session, allow the trace to be selected and
  copied and open its folder; no window after a normal
  close. The screenshots are taken after three frames of the independent
  window. `AURORA_DEMO_CRASH_DIR` makes it possible to use one temporary
  folder per scenario, without touching the real reports. While a
  viewer runs, its session folder holds a `watching` marker; without it
  the watcher did not start (its error is the first line of the console)
  and no window would follow a crash.
  The "Send a crash report" button must open the shared
  form, with the title and the information of this session. To
  capture it automatically, combine with `AURORA_DEMO_BUG_REPORT=1`
  (Description) or `details` (Data). Demo mode stays without sending,
  GitHub authorisation or browser opening. The public excerpt and the
  private ZIP start unchecked. Unchecking a piece of information must remove
  it from the preview. Closing and reopening the form keeps the draft.
- **`AURORA_DEMO_BUG_REPORT=1|details|image|image-ui|direct`**: opens the
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
  select an image and, in Data, explicitly check the private ZIP
  (unchecked at the start), then send. The issue must show the image and the
  link to the logs, with no content of the ZIP in the public text. Downloading
  the ZIP must ask for GitHub and refuse an account without access to the
  private repository `Aurora-Viewer/aurora-api`. An authorised developer must
  be able to open it. After a failed upload of the files, closing and
  reopening the window must keep the same issue and offer to finish its
  upload, without creating a duplicate.
- **`AURORA_DEMO_LOGIN=cards|cards2|cards8|derez|mfa`**: synthetic account carousel with three, two or eight accounts and generated landscape thumbnails; `derez` starts the shortened fragment transition and holds synthetic connection progress inside the selected card, while `mfa` opens the two-factor challenge. No grid connection, saved-scene reads or credential-store access. Other card-mode clicks report that demo mode cannot connect. Cards keep their own grid/start selectors without password controls or a choice heading/count. Add account remains outside the carousel; arrows appear only above three accounts; smaller catalogues remain in a fixed row even in narrow windows. Check 800 × 733 and 1280 × 720 layouts, vertical scrolling at 800 × 560, and the 30-day checkbox in the MFA dialog.

- **`AURORA_DEMO_LOGIN=connecting|first-connecting|add-connecting`**: synthetic waiting state in the selected card, inline first-account form, or Add form. Capture early and late frames (`AURORA_CAPTURE_FRAMES=20|40|90`) to check the brief animation, status/bar placement and persistent Aurora background. No black progress screen and no grid request. Confirm that inputs remain read-only while pending; a real error/MFA/terms challenge must restore the form, and successful authentication must switch to region loading.
- **`AURORA_DEMO_LOGIN=cards1|derez1`**: a single saved account; `derez1` starts its validation. With no neighbour card to shatter, a faint echo of the chosen card breaks apart towards both sides from under it. Capture `AURORA_CAPTURE_FRAMES=30|36|42` and compare with `derez` (three cards).
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
- **`AURORA_DEMO_UPDATE=1|progress|ready|ignored`**, with `AURORA_DEMO=1`: offer
  of a synthetic version, download at 42% or download finished.
  "Update" animates a simulated progress; "Cancel" goes back to the
  offer. No download and no file replacement. Login
  is inert and the login preferences stay intact.
  After "Skip this version" or "Later", the "Check for
  updates" button of the Version panel reopens the offer. The `ignored` mode
  starts directly with the dialog closed to try this flow.
  In a small window, the button stays reachable at the top right.
  `AURORA_DEMO_UPDATE_MAINTENANCE=cache|settings|both` shows the forced effects;
  when absent, the dialog announces the usual preservation and migrations.
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
- **`AURORA_DEMO_CONV=bubbles[:busy|group|local|legacy]`**: the Conversations
  window as a messenger, placed at 700 x 560 px. Without a suffix, the
  conversation with an online friend over two days: their bubbles on the
  left with their picture and a green dot under the last one of each
  group, mine on the right on the violet to indigo gradient, the time under
  each group, "Yesterday" above the first message and "Today" between the
  two days, the corner under the sender square on the last bubble of a
  group; round pictures; the head shows the picture, its dot, the display
  name and the user name. `busy`: a resident in the
  region playing the do not disturb animation, after their automatic
  response: red dot and "Do not disturb" pill in the head, red dot in the
  list. `group`: the demo group chat, the sender's name opening each group
  of bubbles. `local`: the nearby chat (residents, an object with its box
  and its name in its color above its group, the viewer's notices in
  centered pills). `legacy`: the friend's conversation in the legacy lines
  of the setting `chat_style` (`Legacy`; `Bubbles` by default), which the
  demo does not save.
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
  and "Change your environment settings"; Yes says "welcome to the plaza" in
  the chat.
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
  Yes / No), and no toast. `card-answered`: the offer accepted from the
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
- **`AURORA_DEMO_RADAR=1|tp|friends|groups|profile`**: the People window.
  `1`: residents at every range in "Near you" (with
  `AURORA_DEMO_TAGS=1`, those of the name plates, several of whom have a
  profile picture). `tp`: three more residents in a region 10 km away, which
  the demo leaves "by teleport" at frame 600; with
  `AURORA_FPS_LIMIT=60`, they are greyed out at frame 780 and removed from the
  list at frame 1400. `friends` and `groups`: these tabs. `profile`: the
  Groups tab and the "Group Profile" window of the first group.
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
  carrying every badge: the quick actions take the place of the time and
  the range, beside the badges, never over them. `legacy`: the same
  residents in Firestorm's table, for this run only (the preference is
  not changed). Capture `crowd` at frame 900 or later: the profiles
  (payment info, notes) arrive four a second.
  The other tabs, with the mock-up's contents and windows (the contact
  sets, the nickname, the pinned group and the recent people are set for
  the run only, nothing is written to the account's files): `friends`
  (860 × 600: Nova and Orion online, pixelboutique and testtouch offline,
  the rights each way on the rows, Nova's sheet with "Nickname and contact
  sets"), `groups` (860 × 500: "No active group", Aurora Builders active,
  pinned, two notices unread and its sheet open, the market's chat
  blocked, "3 groups out of 42"), `sets` (520 × 430: Family, Clubbing and
  Shop, the wolf's nickname, a non-friend, Nova chosen with her quick
  actions out: move, take out, menu), `recent` (420 × 300: four residents
  met lately, how long ago and where) and `blocked` (420 × 300: a resident
  blocked in full with "Unblock" out, one for voice and text, an object
  for its sounds). Each tab keeps its menu of several people (Ctrl /
  Shift) and its sheet; a window too narrow for the six tabs shows the
  ones around the open tab beside "…".
- **`AURORA_DEMO_STATUSMENU=1|invisible`**: the menu of the status
  dot, open at frame 300. `invisible`: with this status chosen (hollow
  grey dot); only the display changes, no right is modified and nothing
  is saved.
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
  place" greyed out); where they live: `row` (the row under the islands),
  `folded` (the row folded to its tab), `rowdrag` (frame 654: the second
  pill dragged after the fourth), `dock` (the dock's Favorites button);
  with `favorites` the place island's menu or, with `dock`, the grid opens
  at frame 300, the mockup's row or tile under the pointer from frame 700.
  `chatname`: the newest resident's name of the chat pills clicked at
  frame 700, its profile opens (an object's name inspects the object).
  The system island: `systip` (its detailed tooltip shown, the network's
  last minute under its losses and bandwidth), `sysmenu` (its right click,
  under the island at its right end as the mockup draws it, from frame
  300), `netgraph` (Firestorm's traffic indicator shown before the audio
  controls), `sysperf` / `sysnet` (Performance opened from the island at
  frame 300, by its frame rate / its network: the full view 8 px under the
  island, its left and right edges on the island's, down to the work
  area's bottom; the compact view where it stands). In demo the network's graphs show a synthetic minute of
  traffic (about 1.2 Mb/s in, waves), no packet lost. The hidden interface:
  `hidden` (hidden at frame 600 as Alt+Shift+U does: the bars and the
  windows go, the hint at the top in the middle for 6 s; with `legacy`,
  Firestorm's question instead), `peek` (hidden, the pointer held at the
  top edge: the top islands back, no hint under them), `peekleft` (hidden,
  the pointer held at the left edge: the left toolbar back, with
  `AURORA_DEMO_TOOLBAR=side`).
  Capture at
  `AURORA_WINDOW_SIZE` 1280, 1536 and 1920 wide with `AURORA_FPS_LIMIT=60`
  at frame 780: at 1280 the place shows the parcel and the region, at
  1536 the position too, with `rights` the forbidden rights are named at
  1536 and all of them at 1920. With `AURORA_DEMO_NOTIFS=1`, frame 1160:
  the bell's red counter.
- **`AURORA_DEMO_CONTACTS=friends|groups|sets|detached|add`**: Contacts
  tab of Conversations on Friends, Groups or Contact sets (demo contact
  sets and nicknames), the detached Contacts window, or the resident
  picker of "Add...".
- **`AURORA_DEMO_CHATLOG=im|find|history[:find]|offline|long:<n>[:scroll]`**:
  the conversation history without a grid. The demo is its own
  "grid": its files are in
  `config\demo\accounts\<demo account uuid>\chat_logs\demo\`, never in
  the folder of a real account, and the scenario rewrites them at each launch
  (without this variable, the demo writes and reads back no history). The
  windows are placed at a known size.
  - `im`: a conversation with Nova whose file contains two past
    days. What to see: these lines in grey, without a profile picture, with a
    date marker at each change of day, then the new message in its
    normal colours; the file `Nova Example.txt` receives this message.
  - `find`: the same, then Ctrl+F (pointer over the window) and a word typed at
    frame 300. What to see: the
    search bar under the lines, "5 / 5", the matches on an amber
    background and an amber stroke in front of the line of the current match.
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
    `AURORA_DEMO_CONTACTS` seeded one, rights bar, three favorite places,
    two groups of which one in common, private notes);
  - a stranger: `wolf` (blocked: red badge, "Unblock"; "Add as friend" in
    place of the rights bar);
  - my profile: `self`, then `self:edit` (camera button, pencil, About
    editor, Visibility, the unsaved bar once About is changed);
  - compact: `wolf:compact` (icons in a row, the tabs in a drop-down list).

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
  info (four scripted objects, then two attachments in "My attachments"). Roles: `owner` makes
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
- **`AURORA_DEMO_REGION=overview|manage[:owner|:manager|:resident][:preview][:compact][:access|:rules|:terrain][:pending][:ask-…]`**:
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
  (the header names the managers, shows the estate's access and voice and
  the greyed interventions, "View as" offers the role / Resident).
  `resident` gets neither: no People tab, no "View as", no managers nor
  interventions. `preview` opens on "View as" Resident; `compact` opens
  the window 480 px wide (no map, one column, the action buttons as
  icons). People is greyed until a next step. `manage` opens on Manage
  (who can come, region rules, terrain and water; the other sections of
  the navigation are greyed): `access`, `rules` or `terrain` scrolls to
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
  banner and a sentence instead of the estate's access.
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
  enrollment and fee; Save logs `demo group: identity saved`), Abilities
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
- **`AURORA_DEMO_PLACE=1|lagoon|nordheim|grove|flaw|unknown|landmark|history`**:
  "Places" on a place profile, as after a click on a place link.
  `1`: the home of Violet Wolf on Aurora Demo (the parcel of "About
  Land"); `lagoon`: a Moderate parcel; `nordheim`: an Adult
  parcel belonging to a group; `grove`: a parcel with no name,
  description or picture; `flaw`: RemoteParcelRequest answers HTTP 404;
  `unknown`: a region for which nobody answers (error after 10 s);
  `landmark`: the profile of a landmark; `history`: that of an entry of the
  teleport history.
- **`AURORA_DEMO_PLACES=favorites|landmarks|history`**: the Favorites
  and Landmarks folders of the demo (with a subfolder) and a teleport
  history over several months.
- **`AURORA_DEMO_LANDMARK=create|favorite|landmark`**: once the avatar
  stands on its parcel, `create` opens World › Landmark this place: the
  landmark is created at once, named after the parcel; OK renames it,
  gives it its notes and moves it to the chosen folder, Cancel deletes it.
  `favorite` / `landmark` make a landmark of the plaza in Favorites /
  Landmarks, as the star's menu does: the star at the right end of the
  location field turns amber (its tooltip names the folder and the
  landmark), a click on it edits that landmark, and World › Landmark this
  place is greyed out. Without the variable the star is an outline and
  opens its menu. The demo parcel is the 128 m square around the plaza
  (x and y 64..192): a landmark made there lights the star, wherever in it
  the avatar stands. With `AURORA_DEMO_PLACES` the folder list shows the
  demo's subfolder.
- **`AURORA_DEMO_SEARCH=web|people|groups|places|land|events|classifieds[:text]`**:
  the "Search" window on this tab, the search for the text already
  run (`aurora` by default): the list is filled by the directory of the
  demo, the first row is selected and its details shown on the right.
  For `land`, the text is the price limit (`land:25000`); without it the
  field keeps its default "0" and only the auctions (L$ 0) pass
  the limit checked by default.
  A text containing `page` (for example `people:page`) answers a full
  page: "> 100 found" and the Next button; a text containing
  `xyz` finds nothing ("'xyz' not found").
- **`AURORA_DEMO_SEARCH_WEB=<page>`**: the address the Web tab loads in
  demo (a local `file:///…` page is enough to try back, forward,
  reload and the address). Without it the Web tab of the demo loads
  nothing: the search page of the grid is only opened once
  connected.
- **`AURORA_DEMO_MARKETPLACE=<page>`**: World › SL Marketplace stands for
  this page in demo (a local `file:///…` page is enough): the window opens
  at start on it, the menu entry shows, and a link to an address under it
  opens in the window instead of the system browser. Without it the demo
  has no Marketplace (the entry is hidden, as on a grid without one).
- **`AURORA_DEMO_MARKETPLACE_LISTINGS=merchant|not-merchant|error|audit`**:
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
  enabled, health at 72%.

**`AURORA_DEMO_SHAPE=body|head|eyes|ears|nose|mouth|chin|torso|legs|male|readonly|nocopy|dirty|save-as|saving|save-feedback|mesh-head|static-head|pbr-head|mesh-drag|layered-head`**
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

**`AURORA_DEMO_APPEARANCE=gallery|outfits|worn|save|edit|menu|duplicates`**
— opens Appearance with four synthetic outfits:

- `gallery`: the gallery (the right click includes rename, favourites, image
  choice, save to the chosen outfit and move to the Trash);
- `outfits`: the list of outfits; `worn`: the worn items;
- `save`: the Save as dialog; `edit`: the outfit editor;
- `menu`: an expanded outfit with an object that is not worn, for the right
  click menus;
- `duplicates`: replays folder answers with repeated links and
  originals coming from other parents.

These operations stay offline; creating new clothing and
new body parts is not available yet.

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

**`AURORA_DEMO_GESTURES=manager|editor|play|completion|completion-send|communicate|communicate-open`** — three synthetic gestures:
chat with a replacement and F8 shortcut, an animated pose on F9, and an
inactive chime on F10. `manager` opens the list; `editor` also opens the
chat gesture's editor; `play` starts the chat and animation at frame 650.
Capture at frame 620 for the list/editor, 680 during playback and 800
after the explicit pause. Sounds stay muted unless `AURORA_DEMO_SOUND=1`.
`completion` gives the chat gesture the `/eject` trigger, focuses nearby
chat at frame 540 and types `/ej` at frame 550: capture frame 620 to see
`/eject` with the added `ect` selected. `completion-send` also presses
Enter at frame 650: capture frame 800 to see the gesture's chat steps and
the cleared input. Typing replaces the selected suffix; Backspace removes
it without immediately completing it again. Only active, enabled gestures
participate; ambiguous prefixes extend only to their common suffix. The
same completion is available in the conversations window's local chat,
with instant messages and numeric channel prefixes keeping their input.
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

**`AURORA_DEMO_INVENTORY`** — opens a synthetic inventory: personal
folder, subfolder, outfits, objects, documents, texture, sound, gesture,
sky, material, link and varied permissions; library in the tree,
root open and labels aligned left. Creations and mutations simulated
offline; the files and snapshots uploaded in demo stay offline.

- `1`: the inventory alone.
- `delete-key`, `delete-button`: select a folder containing three
  levels and their items, then open the confirmation with the Delete key or
  the trash button of the window footer. Capture 660 (selection and trash to
  the right of the counter), then 720 (confirmation identical to the context
  menu). Delete moves the whole folder to the Trash; Cancel keeps it in place.
  Also check a multiple selection, a worn / protected object (command
  unavailable), a text field (the Delete key edits the text) and a folder
  window. A selection entirely in the Trash asks for confirmation
  before permanent deletion.
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
  (see `AURORA_DEMO_NOTIF` above).
- `folder`, `object`, `animation`, `script`, `note`, `properties`, `nocopy`,
  `notransfer`, `link`: the menu of the chosen item, or its properties.
  `clothes`, `body`, `settings`, `uploads` are aliases of `folder`, to be
  combined with `AURORA_DEMO_POINTER` to hover the submenus.
- `received`, `received-filtered`, `received-missing`: Received purchases tab,
  search limited to its Marketplace box and state with no received folder. The
  list and the Collapse / Expand buttons keep the same presentation; no
  personal folder must appear in this tab.
- `animation-open`, `animation-properties`, `image`, `image-photo`,
  `image-picker`: the five reference panels.
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
- `note-task`, `note-task-readonly`, `note-task-save`, `note-task-error`:
  same editor for a note inside an object. The last two add
  text at frame 650 then send Ctrl+S at frame 670; the refusal keeps
  the draft. Saving and closing / reopening keeps the text during
  the demo session, without creating an item in the personal inventory.
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
| `AURORA_DEMO_SOUND=1` | World, interface and media sounds audible |
| `AURORA_DEMO_VOLUME=open` or `muted` | Volume panel of the top bar pinned open |
| `AURORA_DEMO_VOICE` | Voice states and colours of the two avatar name tags |
| `AURORA_DEMO_CALL=p2p`, `hangup`, `group`, `conference`, `conference-legacy`, `conference-invite`, `incoming` or `incoming-group` | A voice call started or received, with a simulated voice server |
| `AURORA_DEMO_TAGS=voice`, `status`, `cycle`, `wall` or `height` | Name plates in all their states, at the three distances; occlusion and stable height |
| `AURORA_DEMO_FRIEND_PLATES=1` | "Full plates for friends only" for this run, without saving it |
| `AURORA_DEMO_PLATE_HOVER=x,y[,click]` | Pointer held at x, y for the name plates, and a click there at frame 700 |
| `AURORA_DEMO_PLATE_CLICK=profile`, `im`, `tp` or `menu` | "Click on a plate" for this run, without saving it |

### Details

- **`AURORA_DEMO_VOICE`**: `speaking` animates the outline of our own name
  tag and of the other avatar's with realistic synthetic levels (0.35 to
  0.61 on the WebRTC scale), `loud` holds a strong level of 0.60: both
  outlines must be red without needing a saturated signal. `listening` keeps
  both name tags neutral, `off` shows the red struck-through microphone.
  `cycle` chains 4 s of speech, 3 s of listening, 2 s out of voice. Without
  this variable, our outline follows the real microphone when its button /
  push-to-talk is active and voice is enabled. No saved setting is changed.
  With the name plates (default style), the outline is green in the voice
  channel and a tight green halo follows the level; out of voice, the plate
  has neither a coloured outline nor a badge.
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
- **`AURORA_DEMO_TAGS`**: nine residents in front of the starting position
  (`demo_tags.rs`), to see the name plates without a grid. Five are less
  than 15 m from the camera (full plate), two around 27 m (compact pill),
  two around 50 m (name only). A white wall stands behind the two on the
  left, to judge the glass on a light background next to the same plates on
  the dark scene. The People window is closed.
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
    (green outline and halo that follows the level, microphone, green heart
    and triangle badges: he is shown as too complex;
    his name is in ink like everyone's unless Preferences › Colors › "Color
    the names by relation" is on); Lune
    is typing (three animated dots) and has no display name (two lines); La
    Brume is blocked (dimmed plate, name in italics, forbidden badge, no
    status dot). Farther away, Sol is speaking (green outline) and Iris is
    typing; beyond, Vega is in the channel and Atlas is not (the far pill
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
  - `AURORA_DEMO_PLATE_HOVER=x,y` holds the pointer at x, y (points) for
    the plates only: the plate there shows full ("Expand a plate on hover",
    on by default), a pill or a far name included; with `,click` the plate
    is clicked at frame 700. `AURORA_DEMO_PLATE_CLICK=profile|im|tp|menu`
    sets "Click on a plate" for the run (not saved): the pointer turns into
    the hand of a clickable object over another avatar's plate, and the click
    opens the profile, the conversation, the teleport offer box or the
    avatar's menu. For example, with `AURORA_DEMO_TAGS=voice`, `580,316` is
    Orion's plate and `198,334` Nova's pill once the friends-only option is on
    (`AURORA_DEMO_FRIEND_PLATES=1`).
  - The pictures are generated gradients (`demo_tags::picture_pixels`),
    requested as on the grid: profile, then texture, at most four per
    second, the nearest first; our avatar and La Brume have the same one.
  - To be combined with `AURORA_DEMO_VOICE` for our own plate and that of
    Loup Violet (blocked in the demo), and with `AURORA_NAME_TAGS=classic`
    to compare with the classic name tags.
- **`AURORA_DEMO_MEDIA_CLICK`**: focus click at frame 700 by default,
  second click 60 frames after, optional click on the field 120 frames
  after, then "aurora" is typed. `AURORA_DEMO_MEDIA_CLICK_FRAME` changes
  the starting frame to let the plugin load; also works with
  `AURORA_DEMO_HUDS=media`.
- **Media plugins.** The media scenarios need Aurora's media plugins pack
  (SLPlugin, CEF, LibVLC). A viewer that finds no plugins downloads it once
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
  instead of the GitHub release; only a loopback address is accepted, and
  the archive must still have the size and the SHA-256 written in
  `crates/aurora-updater/src/media_pack.rs`. A pack installed that way is
  replaced when these two values change.
- **`AURORA_DEMO_MUSIC`**: `1` designates an unreachable dummy address; the
  music waits for the radio button.
- **`AURORA_DEMO_SOUND=1`**: world sounds (looping chime) and interface
  sounds (taken from the real sound cache if it exists, otherwise a brief
  tick per sound). Without it the media play muted too: the viewer sends
  them a volume of 0, so a video in the demo is silent unless this is set.
- **`AURORA_DEMO_VOLUME`**: `open` keeps the volume panel of the speaker
  open as if the pointer were on it (the panel normally opens on hover and
  closes 0.35 s after the pointer leaves it); `muted` also draws the speaker
  struck through, as after a click on it. Display only: the volumes and the
  mutes of the settings do not change. Look at the panel: only Music,
  Multimedia and Voice have an enable box.

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
| `AURORA_SYNC_SLICE=1` or `alt` | Scene sync by slice of all the objects, as before |
| `AURORA_SYNC_CHECK=1` | What the scene sync visits each frame |
| `AURORA_NAME_TAGS=classic`, `plate` or `alt` | Style of the avatar name tags for this launch |
| `AURORA_LABEL_OCCLUSION=0` | Name tags and floating texts drawn over the scene, as before, instead of behind what is in front of them |
| `AURORA_TEXTURE_BC7=0` | Textures never compressed to BC7 |
| `AURORA_TEXTURE_BC7_CACHE=0` | BC7 without its disk cache |
| `AURORA_RENDER_THREAD=0` | Frames drawn on the main thread |
| `AURORA_EYE_RELATIVE=0` | Vertices of the objects and the terrain computed in region coordinates |
| `AURORA_SHADOW_LEGACY=1` | Shadows drawn by the old path |
| `AURORA_DEBUG_SHADOWS=1` | Shadow view: the cascade each pixel reads, and its shadow |
| `AURORA_PROFILE=1` | Profiling in the log (see [Profiling](#profiling)) |
| `AURORA_PROFILE_FRAMES=1` | Detailed profile of each frame |
| `AURORA_GPU_VALIDATION=1` | wgpu validation layers |
| `AURORA_HOVER_CHECK=1` | Diagnostics of the hover cursor |
| `AURORA_CLICK_PICK=wait` or `check` | Depth under a click read by waiting for the GPU, or checked |
| `AURORA_HOVER_LEGACY=1` or `ab` | Search for the object under the cursor as before the classes and the triangle hierarchies |
| `AURORA_STILL_CHECK=1` | What changes when the image should be still |
| `AURORA_DEBUG_GLOW=1` | Glow diagnostics |
| `AURORA_GLOW_SKIP=<mask>` | Removes a family of faces from the glow |
| `AURORA_MEDIA_DEBUG=1` | Media diagnostics |
| `AURORA_FONT_FALLBACK=0` | Without the system fallback fonts |
| `AURORA_TEXTURE_BUDGET_MB=<n>` | Forces the texture memory budget |
| `AURORA_TEXTURE_SLOTS=<n>` | Size of the texture table, to test a full one |
| `AURORA_BANDWIDTH_KBPS=<n>` | Bandwidth setting for this launch, without changing the Preferences |
| `AURORA_LIGHT_TILES=0`, `alt` or `check` | Local lights without sorting by screen tile, or check of the sort |
| `AURORA_PROBE_WALK=list`, `0`, `alt` or `check` | Reflection probes without sorting by screen tile, or check of the sort |
| `AURORA_BLEND_EARLY_DISCARD=0` or `alt` | Invisible fragments of transparent faces lit before being discarded |

### Details

- **`AURORA_BANDWIDTH_KBPS=<n>`**: the "Maximum bandwidth" of Preferences ›
  Network and cache for this launch, without changing the saved setting,
  from 100 to 10,000. A session first asks its simulator for one and a
  half times the setting, 6,000 kb/s at most in all
  (`aurora-net/src/throttle.rs`): the default, 4,000, already asks for
  that most, and 3,000 is Firestorm's default. To compare what a simulator
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
- **`AURORA_TEXTURE_BUDGET_MB=<n>`**: the texture pages must not exceed
  n MB, whatever the "Texture memory" setting and the video memory the
  driver announces (`scene/texture_budget.rs`). Used to put the viewer
  under memory pressure on any machine, in demo as on the grid.
  - Above the target, the log announces each change of step
    (`texture memory: pressure 0 -> 1 (… MB in pages for a target of … MB, …
    lowered so far)`): 1 brings the textures back to their on-screen size,
    2 to 7 then remove half a level at a time, up to two levels, off-screen
    first. The same line gives the viewer's video memory: its resources,
    what the driver counts for it and the budget granted. The full
    Performance window shows the target and the step ("Texture budget", in
    Memory and streaming).
  - In demo, with `AURORA_DEMO_STREAM=3000` (4.7 GB of textures):
    `AURORA_DEMO_POS="168,126"` places the avatar in the middle of the
    objects; 512 stays at step 1, 96 climbs to step 4 or 5 depending on the
    run. The textures of this scenario are then reloaded on demand, like
    those of the grid (camera moved closer after a reduction). A huge value
    (1000000) removes all pressure, to compare.
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
- **`AURORA_FONT_FALLBACK=0`**: uses only the embedded fonts, as before the
  system fallback fonts were added: the characters they lack (Georgian,
  CJK, Arabic, mathematical letters…) become squares again. To compare with
  `AURORA_DEMO_GLYPHS=1` or to isolate a font problem. The log says
  `system fallback fonts: off`.
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
  place at rest. The line ends with a check of all the objects, made once
  per second: `detail level behind: a of b at rest, c of d placed by the
  GPU` counts those whose requested level of detail is not, at that
  instant, the one their distance gives (a check the sync has not made
  yet); to be read with the camera moving, with and without
  `AURORA_SYNC_SLICE=1`. This check costs one walk over all the objects per
  second. In demo as on the grid.
- **`AURORA_RENDER_THREAD=0`**: draws the frames on the main thread, as
  before the render thread, to compare and debug. By default, the main
  thread hands each frame to the render thread as a self-contained packet
  and moves on to the next frame; with `0`, the same packet is executed in
  place. `thread=on|off` in the `perf summary` line, and a log line at
  startup (`renderer: frames drawn by…`).
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
  thread's CPU time, Windows only), the longest first. A slow step listed
  there waited instead of working: a lock (the heap while the background
  tasks allocate, for example), or a core taken by another thread when the
  machine is loaded. The steps that hand work to the pool (`sync`, `lists`,
  `poses`, `r_finish`) wait there for their threads by construction; those
  that wait by nature (`render`, `limiter`, `r_acquire`, `r_present`) are
  not listed there.
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
    packet.

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
