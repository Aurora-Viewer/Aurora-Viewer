# Webcam tracking prototype

Build › Webcam and Video Motion Tracking animates your own avatar in the viewer. The camera
starts on request and stops when you close the window, log out or quit. The
companion runs inference on this computer and sends landmarks and small
preview frames to Aurora over its private stdout pipe. Camera frames stay in memory. The viewer sends
its normal avatar animations to the grid.

## Try it

1. With Python 3.12 installed, use PowerShell on Windows to create a venv,
   install the tested packages, and launch the demo.

   ```powershell
   py -3.12 -m venv "$env:LOCALAPPDATA\AuroraWebcam\venv"
   $env:AURORA_WEBCAM_PYTHON = "$env:LOCALAPPDATA\AuroraWebcam\venv\Scripts\python.exe"
   & $env:AURORA_WEBCAM_PYTHON -m pip install 'mediapipe==0.10.21' 'opencv-contrib-python==4.11.0.86' 'cv2-enumerate-cameras==1.3.0' 'pillow==13.0.0'
   $env:AURORA_DEMO = '1'
   cargo run -p aurora-viewer -- --title 'Test local-webcam-tracking'
   ```

   Launch the viewer from the same shell so it inherits the interpreter
   path. With the dependencies installed in the default `python` interpreter
   on your PATH, the environment variable is optional.
2. Open **Build › Webcam and Video Motion Tracking**, choose a camera from **Input source**,
   and select **Start camera**. **Refresh cameras** lists attached devices,
   including OBS Virtual Camera on Windows. **Next camera** cycles through
   the devices. Switching inputs during capture restarts tracking on the new
   source and recalibrates the neutral pose. With no named devices, try
   the numbered OpenCV inputs. The camera enumeration package is optional
   for numbered camera input and local replay.
3. Face the webcam with your arms in view. Look forward with your eyes open
   and mouth relaxed, then select **Set neutral pose**. **Mirror motion**
   swaps sides in the preview and avatar. **Front view** selects Aurora's
   front camera preset. Use the channel checkboxes to
   isolate head, eyes, expressions, hands, upper body, or legs and feet. Expression strength
   changes the jaw and eyelid response.
4. Turn and tilt your head, blink, open your mouth, separately lift each arm,
   turn the wrist, and make a fist. Move out of frame, then select **Stop
   tracking** and verify that the usual animation takes over. Use a Bento
   rigged head and hands weighted to the corresponding joints for a
   meaningful in-world comparison.

Choose a demo file or use **Open file…** to track a local video, GIF, or
still image (PNG, JPEG, BMP, or WebP). GIFs follow the frame delays stored
in the file. **Loop playback** repeats a video or GIF. A still image is
analyzed once and holds the detected pose until you stop or switch inputs. Set
`AURORA_DEMO_WEBCAM_VIDEO` before launching the offline demo to preselect a
clip and warm the models during viewer startup. The preview keeps its place
while idle, opening a source, or switching inputs. The preview and tracking
rates are shown separately. File playback previews the first image as soon
as it is decoded, then starts motion and audio after a short lookahead. Video
and GIF motion is resampled at up to 30 updates per second, including frames
between model observations. The preview follows the original pictures.
MP4 audio plays locally through Aurora's Media volume. **Mute video**
silences the clip while playback continues. Clearing it resumes sound at
the current position. The soundtrack follows video loops, source switches,
and playback stops.

### Play a media link

Paste an individual YouTube video or Short, TikTok video, Instagram Reel,
Facebook Reel, X post with video, or direct HTTP(S) media URL into
**Video, GIF or image link**, then select **Play link**. The
first play downloads a temporary local copy before starting the same
preview, audio and motion pipeline used for files. **Stop tracking** or
switching sources cancels a pending download. Replay and loop reuse the
temporary copy while the panel remains open. Each download is limited to
128 MiB. Direct links must return a supported media file.

Video pages use the optional [yt-dlp](https://github.com/yt-dlp/yt-dlp)
package in the tracking interpreter. YouTube may also need Deno. FFmpeg
on PATH can merge separate video and audio tracks:

```powershell
& $env:AURORA_WEBCAM_PYTHON -m pip install --upgrade 'yt-dlp[default]'
```

Deno can be on PATH or configured with `AURORA_WEBCAM_DENO`. The helper
prefers progressive MP4 and uses one video even when a post contains
multiple videos. TikTok short links and Instagram and Facebook share
links can be opened when the platform redirects anonymous visitors to
the video. Availability depends on the platform's anonymous access and
formats. Live streams, login-only videos, profiles and playlists are
outside this mode.

### Control a video

A seekable local or downloaded video gets its own transport under the
preview. Pause or resume, skip by seconds, drag the playhead, or enter a
position in seconds. With the pointer over the transport, Space toggles
playback and the left and right arrows skip three seconds. Choose a speed
between 0.5× and 2×. Speed changes preserve the current position and change
the pitch of the soundtrack along with playback. **Video range** sets the
source boundaries. With looping on, playback returns to the in point.

### Record a take

Start tracking a camera, video, or animated GIF. Open **Record motion** in
the tracking panel and select **Record** once a subject is detected. The
take starts immediately by default. Turn on the optional three-second
countdown when you need time to get into position.
**Finish recording** ends the take sooner than the selected auto-stop
time (one to 60 seconds). The timed recorder samples up to 30 times per
second and follows elapsed time when the input or viewer runs slower.
**Pause recording** keeps the take open while allowing the source to play.
Seek, pause the source, or change video speed and the recorder pauses as
well. Resume source playback if needed, then select **Resume recording**
once tracking returns. The recording omits time spent on hold and appends
the new section to the same take.

Use **Trim in**, **Trim out**, the playhead and **Preview take** to
review the selected interval on your avatar. The take preview supports
its own speed, one-second jumps and 1/30-second steps. **Loop take** controls
preview and the exported asset. **Save .anim…** writes the selected
animation locally. **Preview & upload animation…** opens Aurora's existing
animation inspector with that same export, including its ordinary
explicit upload action.

The take contains retargeted joint rotations from enabled webcam
channels, including available foot stabilization. It contains no video,
audio, avatar root trajectory, or rotations from the running animation
override. It uses animation priority 4. The exporter reduces redundant
keys before checking Second Life's 250,000-byte limit. Trim a take
further if it exceeds that limit.

### Export the soundtrack or assemble a gesture

File-backed takes use FFmpeg on PATH to read the source audio. The
soundtrack follows the take's trim, playback speeds and recorded sections.
Muting playback affects monitoring. A WAV export is aligned mono,
44.1 kHz, 16-bit audio. Camera input records motion only.

**Create Gesture in one step** shows the current selection duration. It
prepares a non-looping animation and the soundtrack if **Include soundtrack**
is selected. The confirmation displays the total for the whole batch,
including L$0 when the grid quotes a free upload. Enable **Preview assets
before upload** to review the same batch in Import first. After the selected
uploads succeed, Aurora saves the Gesture with the animation, sound pieces
and required waits, and opens the editor to let you preview it or set a trigger.

**Export or upload individual files** opens the animation and sound previews
separately. Sound previews audition the first piece and uploads split long
selections into up to 29-second pieces. **Save animation and sound…** writes
matching `.anim` and `.wav` files to a folder you choose.

Default file and inventory names start with `Webcam Tracking` and a short
source label, local capture date/time and unique take suffix. Media links use
a platform and shortened public video identifier when available, or the host
name. The generated inventory names fit within 63 ASCII bytes, including
audio segment suffixes.

Gesture assets are separate inventory items and use the usual asset
permissions. The grid plays clips as separate sound events, so later chunks
can be affected by asset loading or timing on the receiving viewer.

### Edit a recorded take

Open **Take tools** below the take preview controls. The marker strip
shows missing observations on commonly tracked joints and any beat
markers. Click it to move the playhead. **Repair short gaps** uses
observations before and after a gap and rejects isolated jumps. **Repair
at playhead** reruns this over a short window chosen by the user. Choose
a joint (for example, mWristLeft) and a width, then select **Repair
selection** to limit the interpolation to that joint and window.
**Restore original** restores the recorded take. The repair operates on
recorded joint rotations and estimates neither new depth nor long unseen
actions.

**Find loop** looks inside the trim range for similar poses and rotation
directions, favoring leg alignment and matching tracked foot contacts.
It sets the new range and blends the final fraction of a second in both
preview and export. When the same foot is detected as planted at the
start and throughout the bridge, the ankle stays anchored during the
blend. **Whole take** resets the trim range.

With FFmpeg on PATH, **Find audio beats** estimates rhythmic accents
from the source soundtrack during the recording. Local video files and
downloaded clips can supply sound. Silent footage and webcams can use
**Place BPM grid** with the playhead on the desired beat. Set the number
of beats per clip and select the current segment or save all segments as
separate .anim files. **Save frame as pose** exports a one-second held
pose from the playhead. The exported assets contain joint motion only.

The prototype estimates head motion, gaze, blink, jaw opening, chest and
arm rotations, palm orientation, finger curl, pelvis rotation, and hip,
knee and ankle rotations. Keep the hips, knees and feet in the frame for
lower-body tracking. File tracking fills brief gaps using observations on
both sides of the occlusion, within the same playback loop. It can briefly
carry an out-of-frame wrist relative to an observed elbow. Live input keeps
its low-delay, forward-only prediction. Head rotation is relative to the
torso. Body head points provide a fallback when the face mesh loses a
profile, and side views favor the bend visible in the source image.
When a file shows a stationary foot near the bottom of an upright pose,
two-joint leg IK holds the ankle relative to the avatar for the detected
stance. Root motion, terrain support, jumps and hand contacts are outside
this approximation. A normal OBS window capture can show the result.

An optional local **RTMPose-s** ONNX model adds a **2D pose** fallback for
files such as stylized animation. Use the [OpenMMLab RTMPose-s model archive](https://download.openmmlab.com/mmpose/v1/projects/rtmposev1/onnx_sdk/rtmpose-s_simcc-body7_pt-body7_420e-256x192-acd4a1ef_20230504.zip)
and set `AURORA_WEBCAM_PLANAR_MODEL` to the extracted `end2end.onnx` path
before launching. The fallback expects a single, well-framed character and
supplies body joints in the image plane. It follows a well-detected subject
with a padded inference crop, and the preview draws links between pairs of
available joints. Depth, toes and face detail are unavailable in this mode.
The companion uses the existing OpenCV runtime.

## Implementation and inspiration

The optional Python companion keeps MediaPipe Pose, Hands and Face Mesh in
memory while the panel is open. Decoding and inference run independently,
with one pending inference frame that newer input can replace. Aurora
smooths new landmark targets across render frames and applies
tracked rotations to its own avatar's render skeleton after blending normal
animations and before composing the joint palette. The Poser and animation
editing tools retain priority. The underlying animation keeps running.

Thanks to [@miyumiyuna5](https://x.com/miyumiyuna5/status/2116499758420512852)
for demonstrating face and upper-body tracking in Aurora. This implementation
was developed independently from that public demonstration.

Linden Lab's [Puppetry documentation](https://wiki.secondlife.com/wiki/How_Puppetry_Works)
and [LEAP webcam examples](https://github.com/secondlife/leap/tree/c6aac3e2df8ca30e845864a62b8cecdc94bf8e05/puppetry/webcam)
describe the separate experimental viewer/region stack. The Aurora preview
uses its own local render layer and requires no region feature.

LEAP is an open client-side plug-in interface. A viewer launches a child
process and exchanges length-prefixed LLSD messages over its standard
streams. The official Puppetry example sends joint targets over that
interface. The experimental viewer's Puppetry module turns them into
animation updates. A capable region must relay those updates for other
clients to draw them. Linden Lab publishes the viewer and the LEAP
library. The simulator source is outside that release.

[OpenSimulator](https://github.com/opensim/opensim) is an independent BSD
licensed server that can run on Linux. An isolated grid experiment would
need an opt-in server capability and corresponding viewer support for
transmitting, receiving and applying joint updates. Installing the LEAP
library by itself changes only the local plug-in side.
