# Motorcycle hand contacts

The motorcycle uses the same [IK Edit rig system](IK_EDIT.md) as every other
creation. The driver and compatible observing viewers receive definitions
and solve them locally for the rider's own shape and animation. Ordinary
viewers continue displaying the underlying animation.

## Install and calibrate

1. Name the target links **Left** and **Right**, with one of each name.
2. In **Build > IK Edit > Bindings**, select each wrist, bind it to its grip,
   and position/rotate the bone to author its target-relative offsets.
3. Click **Copy complete IK script** and replace the contact companion's
   entire source in the motorcycle root. Keep the vehicle's main script.
   [motorcycle-hand-ik.lsl](../scripts/lsl/motorcycle-hand-ik.lsl) provides
   a starting configuration; the editor export contains your calibration.
4. Keep `EXTERNAL_CONTROL = TRUE` and have the main script respond to `sync`
   on channel `-190018108` with its current profile or `off`.

The starting motorcycle declaration contains `drive` (both wrists) and
`gas_only` (right wrist). Both reuse the same binding data; changing profile
does not duplicate offsets or restart a contact that is common to both.

The viewer automatically installs a separate **#Aurora Bridge IK** script.
On region arrival it discovers occupied/worn publishers once using a negative
regional channel. The motorcycle announces as soon as somebody sits and returns
its definition even before the engine's IK profile is enabled. Full definitions
are sent only to a requesting bridge; state changes are small regional events.
There is no periodic observer search or 25-second renewal in live sessions.
Replace the complete motorcycle companion with a new export for this transport,
preserving the calibrated `RIG`. The main script's profile commands stay the same.
To recover an editor project without sitting, choose **Create a new IK project >
From an object or attachment** and click the motorcycle. The current companion
answers that explicit request even while unused; the viewer creates a separate
project without changing the motorcycle's active profile. Existing companions
need the updated source for this authoring request.

```lsl
// Ready and riding:
llMessageLinked(LINK_THIS, -190018108, "profile drive", NULL_KEY);
// After choosing a one-handed pose:
llMessageLinked(LINK_THIS, -190018108, "profile gas_only", NULL_KEY);
// End of the round:
llMessageLinked(LINK_THIS, -190018108, "off", NULL_KEY);
```

The main script chooses its IK mode independently of any animation. It does
not look up animation asset UUIDs or require animation inventory permissions.
On `drive` -> `gas_only`, the left hand fades out and the right hand stays
attached. On return, only the left hand begins fresh entry conditions. A
source counter per binding also covers intermediate switches missed by an
observer. Repeating the same keyword retains current contacts. A complete
off/on starts a fresh global lifetime. Static data remains cached across modes.
Use `EXTERNAL_CONTROL = FALSE` only to start the first profile automatically
while seated/worn. IK Edit itself stays independent of these commands.

Keep the main script's `llStartAnimation("accel_F")` at the existing throttle
event. All viewers receive exactly the same animations. Aurora applies IK
after normal animation blending; no fallback animation is filtered or stopped.
Rotate `Right` at that same event so its bound wrist follows the moving grip.
[motorcycle-throttle.lsl](../scripts/lsl/motorcycle-throttle.lsl) can consume
the existing engine `tap`/`stop` messages on channel 17001. Use it only if the
main script does not already embed the throttle helpers. It applies an
absolute local rotation from a saved rest pose, pulses for 0.46 seconds,
and restores the rest pose on stop or reset. `THROTTLE_DEGREES` controls the
angle; zero `THROTTLE_AXIS` selects the longest local prim axis. A helper
prim may need an explicit axis. The rest rotation is stored per prim UUID
in linkset data; remove its `aurora.throttle.rest.<UUID>` entry before a
reset if intentionally recalibrating the grip's neutral rotation.

Offsets are metric in the target prim's axes; prim scale does not resize
them. The optional contact point is bone-local before avatar hand scaling.
Zero targets the wrist pivot. Follow rotation uses the calibrated quaternion;
position-only follows the grip while retaining animated wrist articulation.
The solve respects shaped/mesh-overridden bone lengths and can use the
selected chain root for torso assistance. Finger closure remains animated.

Standing, losing/deleting the source, detaching, teleporting, region changes
and logout release the applicable sessions without a final off message.
Unused creations cannot populate a rig catalog: only occupied seats and worn
attachments are queried, with definitions held temporarily during that use.
Each definition is limited to 8 KiB; aggregate data, including partial transfers,
is limited to 32 KiB per avatar and 128 KiB for the current region. Inactive
announcements, unsolicited data and excess traffic are discarded. Leaving the
seat, removing the source or relay and changing region drop negotiated data.
The [shared protocol contract](IK_EDIT.md#compatible-observers-and-lifecycle)
describes event discovery, nonce/revision validation and the limits of detecting
a silent stopped script without polling. There is one supported wire protocol,
`auroraIK/5`.

Offline checks use `AURORA_DEMO_HAND_IK=1|moving|release|observer|observer-release`.
Real LSL compilation and two-viewer motorcycle behaviour require human
validation; agents never connect to a grid.
