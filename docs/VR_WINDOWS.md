# Experimental Windows VR scene viewing

The Windows build enables the same native Vulkan/OpenXR implementation as Linux:
stereo scene viewing, camera anchoring, head tracking, toolbar entry, Esc exit,
desktop camera restoration and the desktop mirror. **Windows runtime/headset
validation is pending.** A successful build is not proof of working headset output.
The validated configuration remains Quest 3 / WiVRn 26.9 / RTX 3090 on Linux.

For shared controls, limits and other platforms, see [VR setup](VR.md).

## Requirements

- Windows x64 and a GPU/driver supporting Vulkan 1.3 and Aurora's normal rendering
  features, including bindless resources. OpenXR and the desktop must use a GPU
  compatible with both headset rendering and window presentation.
- A configured active Windows OpenXR runtime supporting the intended headset and
  `XR_KHR_vulkan_enable2`. Select the runtime through its vendor's supported tool.
  An OpenXR badge alone does not guarantee Vulkan interoperability.
- Nothing else to install: the Khronos OpenXR loader is built into the viewer
  (the `openxr` crate's `static` feature), and it finds the runtime the system
  has active.
- `PRIMARY_STEREO` with two views, `LOCAL`/`VIEW` reference spaces, opaque
  composition, compatible RGBA8/BGRA8 eye formats and swapchain images usable as
  color attachments, sampled textures and transfer sources.
- Runtime running and headset connected when the viewer starts. A headset
  connected later makes the headset button offer "Restart in VR": the viewer
  restarts with `--vr` and enters VR once the world shows.
  WiVRn is the Linux test route, not a Windows requirement. No OpenVR bridge,
  VR framework or streaming client is bundled with Aurora.

Tested by a maintainer on 2026-11-01: RTX 4090, Virtual Desktop's OpenXR
runtime (VDXR). Other runtimes are expected to work but are untested. The
runtime selects the Vulkan GPU; a conflicting explicit Aurora GPU preference or
hybrid-GPU configuration may prevent initialization. Attaching a headset
without a restart, headset reconnection and runtime-loss recovery remain
unfinished.

## Build and CI

Use the normal Windows environment described in `HUMANS.md`; build in the task
worktree with its default target directory and dev profile:

```powershell
cargo build --locked -p aurora-render --example xr_spike
cargo build --locked -p aurora-viewer
./scripts/check.ps1
```

The existing Windows workspace/all-target Clippy of the CI compiles the enabled
Windows path. Headset tests use a local dev build.

## Maintainer validation checklist

1. Start/connect the intended Windows runtime and headset. Run
   `target/debug/examples/xr_spike.exe` (or the downloaded `xr_spike.exe`).
   Confirm the sphere/checkerboard scene, stereo depth and head tracking in
   both 15-second sessions. Keep the process running through both cycles.
2. Run the offline viewer first:

   ```powershell
   $env:AURORA_DEMO = "1"
   cargo run -p aurora-viewer -- --title "Test vr-scene-view Windows"
   ```

   With a downloaded binary, set the same environment variable and run
   `./aurora-viewer.exe --title "Test vr-scene-view Windows"` instead.
3. Place/pitch the desktop camera downward. Click the headset button in the
   bottom toolbar. Confirm horizon-level entry, correct stereo/head tracking,
   scene materials/lighting/transparency and no interface/HUD/nameplate overlays.
   Repeat with the headset tilted at activation, then worn level.
4. Check the desktop left-eye mirror, letterboxing and desktop-only Esc hint.
   Press Esc with Aurora focused; confirm original desktop camera/UI restoration.
   Repeat several cycles without restarting.
5. Test focus changes, resizing, minimization and clean shutdown. Check startup
   with no loader/runtime/headset: normal desktop operation should remain
   available; VR is unavailable. Disconnect/runtime restart are separate tests,
   not supported recovery promises.
6. Only after offline tests pass, the human may clear `AURORA_DEMO`, launch
   normally and perform login/live-grid validation. Agents do not log in or
   operate on a grid. Record runtime/headset/GPU/driver versions, results and
   frame-time/memory observations without credentials or personal location data.

## Deliberate limits

**Where the view starts:** Preferences › Camera › VR, "Headset view". By
default ("First person") at the avatar's eyes, as in mouselook, with the head
and what is worn on it hidden; "Camera position" starts where the desktop
camera was. Either way the heading is kept and pitch/roll are levelled to the
horizon, and the view stays anchored there.

**Viewing only:** head tracking permits looking around and small
physical leaning, not locomotion or camera relocation. No supported VR menus,
chat, inventory, building, object interaction, controllers, hands or teleporting.
Exit with Esc to interact or reposition the desktop camera. Esc needs viewer
keyboard focus; headset controllers cannot exit VR.

Each eye is capped at 1600 x 1600. Stereo backends add GPU time/memory; the
30 FPS desktop mirror reuses an eye image instead of rendering a third scene.
No Windows performance, recording overhead or disconnect-recovery claim is made.
macOS/visionOS remain outside this implementation (a later step).
