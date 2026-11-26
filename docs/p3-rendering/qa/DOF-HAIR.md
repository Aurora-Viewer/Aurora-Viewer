# Alpha-blended hair and depth of field

Run these checks from the fixed viewer's source directory with Python 3.12, wgpu-py 0.33.0, NumPy 2.3.5 and a Vulkan adapter. The default run uses the candidate shaders for both routes and explicitly reconstructs how the previous AO-based focus path selected depth. If you have the earlier P3 test kit at `dd99f509a`, add `--baseline PATH` to read its shaders for the previous route. An upstream checkout before P3 lacks the required pass definitions.

```sh
python docs/p3-rendering/qa/dof_hair.py . --json dof-hair-results.json
cargo test --locked -p aurora-render blended_focus_depth_follows_the_active_frame_for_every_aa_mode
```

The scene places a focused alpha-textured card against a distant checkerboard. Curved strands span several opacities and include one-pixel wisps, transparent gaps and a strip behind a nearer opaque surface. The fixture runs Aurora's blended depth fragment code and five lens passes. It reads back depth and linear RGB for individual surface classes. A separate two-pixel test exercises the exact 10% coverage boundary, one-sided backfaces and two-sided cards. The before case uses the previous AO-based depth contract. The before/fixed comparison therefore runs from a single checkout. The optional historical input also checks whether its shader implementation changes that comparison.

The native None, FXAA and SMAA cases share their pre-lens colour and depth path. FXAA and SMAA filter later in the viewer. The TAA case runs its reset and history passes before the lens. The MSAA cases use an actual colour attachment and hardware resolve at the highest supported sample count within each requested setting. The FSR cases exercise the display-depth shader at its five configured ratios. FSR colour is held constant to isolate the depth input. The Rust test verifies the renderer's choice of focus depth and MSAA fallback. The synthetic image does not replay an animated avatar or a full viewer frame.

| Measured quantity | Previous native path | Fixed native path | FSR reference |
| --- | ---: | ---: | ---: |
| Tested detached strands at hair depth, effective alpha from 10% to below 88% | 0% | 100% | 100% |
| Mean absolute change to those strands' linear RGB from the lens, channel range 0–1 | 0.235 | 0.000 | 0.000 |
| Clear gaps retaining background depth | 100% | 100% | 100% |

These results held on SwiftShader and llvmpipe. Both exposed 1× and 4× for the required formats. Requests for MSAA 2× used 1× and requests for MSAA 8× used 4×. A GPU exposing native 2×/8× can execute those counts with the same command. FSR Quality, Balanced, Performance and Ultra Performance kept display-sized depth, including odd image dimensions. The fixture also checks behavior when the optional focus proxy or FSR's blended motion pass is disabled.
