# Experimental VR scene viewing

Place the desktop camera where you want to look from, then use the bottom-toolbar
headset button. VR preserves that position and horizontal heading, levels desktop
pitch/roll to the horizon, and uses headset rotation and small physical movements.
Press **Esc with Aurora focused** to restore the original desktop camera and UI.
Repeated normal entry/exit does not require restarting the viewer.

**Viewing only:** there are no supported VR menus, chat, inventory, building,
object interaction, locomotion, camera relocation, controller or hand controls.
Exit VR to interact or reposition the camera. No viewer UI, nameplates, worn HUDs,
selection outlines or editing overlays appear in the headset. The desktop is a
left-eye mirror with an exit instruction, not an interactive second viewport.

## Platform status

| Platform | Graphics binding | Build/automated evidence | Human headset evidence |
|---|---|---|---|
| Linux | Vulkan / `XR_KHR_vulkan_enable2` | Full local checks pass | Quest 3, WiVRn 26.9, RTX 3090: stereo, tracking, horizon, repeated entry/Esc exit, restoration, mirror and hint validated |
| Windows x64 | Vulkan / `XR_KHR_vulkan_enable2` | Native strict Clippy, viewer/sample builds and shared VR tests passed in a completed one-off fork run | Maintainer is testing a local build; results pending |
| macOS Apple Silicon | Metal / `XR_KHR_metal_enable` | Native binding implemented; existing macOS CI and runtime tests pending | No Mac/headset available in this session; no headset or simulator result claimed |

No platform status means every OpenXR headset works. The runtime must expose the
required graphics binding, device, stereo views and eye image format. Build, unit
tests, simulator rendering and physical headset streaming are separate gates.

## Common requirements

- A GPU/driver meeting Aurora's existing renderer feature/resource requirements.
  The runtime-selected GPU must also be suitable for the desktop window.
- Installed native Khronos OpenXR loader and configured headset runtime. Runtime
  and headset must be ready **before Aurora starts**. VR is optional: unavailable
  loader/runtime discovery falls back to ordinary desktop startup.
- Head-mounted display, two-view `PRIMARY_STEREO`, `LOCAL` and `VIEW` reference
  spaces, opaque composition and the selected RGBA8/BGRA8 eye format.
- Eye images supporting color attachment and sampling; Vulkan also uses
  transfer-source access. Eye dimensions are currently capped at 1600 x 1600.

Late headset attachment, runtime restart/disconnect recovery, broader stereo
visual validation and performance/memory benchmarks remain unfinished. A restart
may be needed after runtime loss. Esc is not a global shortcut and controllers
cannot exit VR. Two eye backends add GPU time and memory; no frame-rate guarantee
is made. No standalone Android/visionOS Aurora application is included.

## Linux

Install the distribution OpenXR loader (`libopenxr_loader.so.1`) and a compatible
Vulkan 1.3 runtime. Start WiVRn/server and connect the headset before launching.
Use the runtime's supported active-runtime registration. Normal use needs no demo
variable. Other Linux runtimes/headsets/GPUs remain unvalidated.

## Windows

See [Windows VR setup and maintainer checklist](VR_WINDOWS.md). Install an x64
OpenXR loader (`openxr_loader.dll`) and select a runtime supporting Vulkan-enable2.
Start with the two-session `xr_spike` sample, then test the offline viewer. The
redundant dedicated Windows workflow was removed following maintainer feedback;
existing Windows CI covers compilation. Headset tests use a local dev build.

## macOS: runtime investigation and native Metal path

Aurora retains its Metal renderer. It does not switch to Vulkan/MoltenVK. The
OpenXR binding queries the runtime's Metal device, checks adapter identity, binds
the exact wgpu command queue, and imports retained runtime-owned `MTLTexture`
images. Session teardown keeps the queue/images alive until rendering completes.
`openxr 0.22` provides raw Metal extension bindings but no typed Metal graphics
adapter; the small private adapter in `xr_metal.rs` uses those existing bindings.

Sources checked on 2026-11-02:

| Source | What it establishes | What it does not establish |
|---|---|---|
| [Khronos Metal binding](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XR_KHR_metal_enable.html) | Ratified native Metal device/queue/swapchain interface | Availability or quality of a particular headset runtime |
| [Khronos runtime inventory](https://github.khronos.org/OpenXR-Inventory/runtime_extension_support.html) | Meta XR Simulator on macOS advertises Metal; a simulator route exists | Physical headset streaming or Aurora compatibility |
| [OXRSys](https://github.com/demonixis/oxrsys/tree/82ee9ff58f26dcbe3d75f94de4357a2df1906b3e) and its [support matrix](https://github.com/demonixis/oxrsys/blob/82ee9ff58f26dcbe3d75f94de4357a2df1906b3e/docs/support-matrix.md) | Experimental macOS runtime source with Metal and headset-streaming clients | Aurora launch, stereo correctness or physical-device qualification; its matrix distinguishes these from builds |
| [WiVRn macOS fork](https://github.com/kzahel/wivrn-macos/tree/0ed2634ad917749b58e23a58b452299d0b27b4ab) | Experimental Apple Silicon host packaging with a native Metal frame probe and Quest route | Upstream WiVRn macOS support or an Aurora headset test |
| [Upstream Monado](https://monado.freedesktop.org/) | Published platform coverage differs from these experimental forks | An upstream production-ready macOS installation |

These are candidate development runtimes, not Aurora-certified products. It
would be inaccurate to say no macOS OpenXR runtime exists; it would also be
inaccurate to infer physical headset compatibility from these repositories.
Windows runtime names are not macOS installation instructions.

### Mac setup and validation

1. Use Aurora's normal Apple Silicon/Xcode environment. Install a candidate
   runtime, its matching headset client if needed, and an arm64 Khronos OpenXR
   loader. Runtime registration remains the runtime provider's responsibility.
   Meta's [simulator setup](https://developers.meta.com/vr/documentation/native/xrsim-getting-started/)
   provides a separate desktop-simulator route; record simulator results as such.
2. If the loader is not discoverable as `libopenxr_loader.dylib`, set
   `AURORA_OPENXR_LOADER` to its absolute installed path. For a development
   runtime, `XR_RUNTIME_JSON` may select its manifest. No loader/runtime is bundled.
3. Validate the runtime independently with Khronos `hello_xr -g Metal`. Aurora's
   `xr_spike` example remains a Vulkan-only Linux/Windows interoperability test.
4. Start/connect the runtime and headset, then run the timed full-scene offline
   test (environment values below are examples of controls, not installed paths):

   ```sh
   AURORA_DEMO=1 AURORA_DEMO_XR_WORLD=1 cargo run -p aurora-viewer -- --title "Test vr-scene-view macOS"
   ```

   This enters after frame 600 and submits for about 15 seconds. Then test manual
   toolbar entry/Esc exit with `AURORA_DEMO=1` and no `AURORA_DEMO_XR_WORLD`.
5. Record Mac/GPU, OS, runtime commit/version, loader, simulator/headset/client,
   stereo/depth/tracking, horizon leveling from tilted desktop/headset entry,
   repeated entry/Esc exit, desktop restoration, mirror/hint, focus, resizing and
   shutdown. Test unavailable runtime and disconnect separately. Only a human
   logs in for any later live-grid validation.

**Pending:** macOS CI compilation and Metal pipeline/demo checks, simulator
submission, physical headset streaming, runtime/driver resource-lifetime
validation, latency/performance and recovery. Maintainer review must decide
whether the experimental runtime ecosystem is sufficient for platform parity.

## Dependencies and footprint

The new runtime dependency is `openxr 0.22.0`, with `openxr-sys 0.14.0` and
`libloading 0.9.0`; the older libloading remains for existing users. Licenses are
MIT/Apache-2.0 and ISC. macOS directly reuses `objc2 0.6.5` and the existing
`objc2-metal 0.3.2` bindings already used by wgpu; no additional package versions
or Metal/Vulkan translation library are added for the Mac path.

No VR engine/framework, runtime, streaming stack, headset client or recording
software is bundled. The system loader/runtime are external requirements. The
interoperability example is not part of the viewer executable. Binary-size,
GPU-memory and frame-time differences have not been quantified.

## Recording

The desktop mirror samples the submitted left eye at up to 30 FPS, with
letterboxing, rather than drawing a third world view or reading it back to the
CPU. Capture the Aurora window with OBS (PipeWire on Linux/Wayland; the native
window source on other systems), using hardware encoding where available. The
exit instruction is included. Capture/encoding adds overhead and the mirror
cannot show headset streaming/compositor-only artifacts.
