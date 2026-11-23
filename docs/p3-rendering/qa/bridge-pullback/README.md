# Glasses bridge pullback regression

From the root of an Aurora checkout containing this change, use Python 3.10 or newer, Rust 1.100.0 and a Vulkan adapter.

```console
python docs/p3-rendering/qa/bridge-pullback/run.py .
python docs/p3-rendering/qa/bridge-pullback/run.py . --tier high
```

The first build downloads the pinned Rust crates and the viewer's wgpu fork. Python uses its standard library. Pass `--json results.json` to save measurements, `--capture raw.json` for the individual receiver samples, or `--measure` when recording a pre-fix version expected to fail. On Linux, SwiftShader and lavapipe can run without a display. SwiftShader's shader compiler may need `ulimit -s unlimited` in the shell. Both the viewer and the test use a matching wgpu release.

The camera pulls back from 16 to 80 cm and crosses the portrait-map handoff. A fixed curved receiver models the nose. A bent, 1.8 mm opaque metal bridge sits 9 mm above it under a fixed directional light. The camera has a 60° vertical FOV and a 640 × 400 depth buffer. The receiver grid stays fixed in world space throughout the run. For each point, 256 analytic light rays independently determine if the bridge physically blocks a soft emitter. Only receiver points visible to the camera at every pose enter the metrics.

The report gives the raw Bend mask, spatially filtered mask, directional map, and composed sun visibility separately. The fixture executes the specified checkout's WGSL for all four. An opaque caster and receiver are rasterized into the graded, portrait, and distant light-depth targets. Contact input is the analytic first surface along each camera pixel's ray. The probe invokes the viewer's avatar receiver path and composition function. Material shading, animation, alpha, asset LOD, temporal reconstruction, scheduling, and GPU performance are outside this test.

**False gap occlusion** is the percentage of directional light removed from the middle 70% of the physically lit interval between the cast shadow and the bridge. **Median width above 10%** measures how much of that interval becomes visibly shadowed, in millimetres. **True core occlusion** checks the region where all 256 reference rays hit the bridge. Thresholds check suppression of false coverage and retention of the real shadow. The final subpixel poses have a looser composed-visibility budget to allow for spatial filtering.

The scene and Vulkan setup are adapted from the P3 close-rim validation fixture. The reconstruction reads current WGSL, dispatch code, and projection helpers from the checkout. No prior fixture archive is required.


## Other synthetic depth captures

Use `--depth-grid DIRECTORY` to replay 384 × 288 reverse-Z frames through the 120-sample Bend tracer and production filter. The vertical field of view is 60°, the near plane 0.01 m. Each `.raw` begins with three little-endian f32 components of the unit camera-space vector towards the light, followed by one little-endian f32 depth per row-major pixel. The resulting `.bend` contains the filtered occlusion byte of each pixel. The corresponding analytic lit-plane and cast-shadow tests are under `docs/p3-rendering/qa/` in the checkout.
