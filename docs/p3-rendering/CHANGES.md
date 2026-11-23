# P3 rendering change register

The original P3 port targeted Aurora `v0.9.13-beta.20261121.2` (`231c88a3`). Subsequent P3 builds used main at `2d505f2b`. The GPU tests were repeated on `7be6c4cd`. The submission also incorporates the signed-updater changes in upstream `2702bb89`. It covers shadows, materials, ambient occlusion and depth of field. The shadow changes apply to the enabled stock shadow tiers. Materials and DoF introduce opt-in experiments, and the GTAO control starts at zero.

## Shader compilation during login

- Stochastic transparency builds its optional GPU pipelines on a worker when the first eligible blended frame arrives. Frames use the existing sorted draw path until a complete set for the current sample count is ready.
- Live Bend contact shadows use the existing 60-sample pipeline while a worker prepares the requested reach. The renderer caches completed variants by their exact sample settings. Explicit captures request the matching kernel before drawing.
- Stall snapshots record known pipeline work on the affected render thread. The window continues to offer Wait or Close for persistent stalls, and confirmed device loss follows the existing recovery path.

## Camera-dependent contact bands on mesh surfaces

At a grazing light angle, the Bend contact tracer could count another sample of the same lit triangle as a blocker. The resulting bands could move when the camera turned, even on a single plane without a caster. The tracer now estimates the receiver plane from opposite depth neighbours and checks whether a candidate at the sampled texel can rise into the light ray. This excludes false hits from coplanar triangles and from convex surfaces curving away from the ray. Sky boundaries, discontinuities and back-facing planes retain the existing hit test. This also applies to the shader variant used on RX 9000 cards and to capture kernels.

The CPU dispatch and shader now use the same pixel for the projected light origin. This covers the far viewport edge when the light projects just outside the opposite edge.

[Unoccluded grazing planes and convex mesh facets](qa/grazing_planes.py) · [Wire, welded ridge and raised step](qa/grazing_casters.py)

## Glasses bridge shadows and stochastic transparency

- On rays approaching the camera, Bend narrows the hit tolerance when depth samples resolve a detached front surface. The spatial filter smooths that portion of the contact mask over a smaller area. This preserves more of the lit gap beneath thin frames during a camera pullback.
- Stochastic transparency checks GPU errors while creating its resources and pipelines. If the sample-mask depth pipeline fails, the renderer tries sample-index depth. If initialization still fails, the viewer draws the existing sorted transparency pass and can retry after the sample count changes or the option is re-enabled. The Vulkan sample-mask module contains only its vertex and fragment entry points and their dependencies.

## Shadow stability changes since the first main port

- The three-level layout now interpolates the receiver's world-space normal offset across the cascade transition. Both maps sample that same point. Previously, their independent offsets could draw two displaced silhouettes from one thin caster. Each map keeps its own depth bias and filter.
- A remote soft-shadow blocker can seed the search from any of the four texels supporting the central bilinear sample. For a hard light the shader reads central coverage directly. Shared fractional coverage in the local and remote depth layers now counts once per sampled light direction.
- P3 contact shadows use a dedicated depth input for half-covered blended casters. Eligible blended receivers use the same 0.50 opacity threshold, and both Bend and the view-space tracer consume that input. Ambient occlusion retains Aurora’s separate opacity-aware depth. This matters for a partially transparent rim in front of skin.
- The normal projector path again uses the P3 five-tap filter. The optional soft projector path falls back to the five-tap lookup when its blocker search misses thin geometry. Set `AURORA_SOFT_PROJECTORS=1` before launch to opt in when soft lighting is enabled.

[Shadow validation and scene checks](VALIDATION.md)

## Review order

1. Start with the overlapping-caster and glasses-rim cases below. These motivated the changes to visibility composition, spatial resolution and transitions.
2. Check the stock tier table, then toggle Custom, switch tiers and reload a named graphics profile. The port deliberately chooses P3's saved settings behavior on the beta.
3. Review the three optional modules independently. Materials affects direct lighting and authored occlusion. GTAO affects screen-space occlusion. DoF affects focus depth and image compositing.

The port builds on the beta's existing renderer, including its Bend tracer, XeGTAO implementation, opacity-aware depth, projector shadow support and Phototools. An underlying feature listed here can therefore predate the port. Each section describes what P3 changes. [Feature controls](README.md) · [Visual validation](VALIDATION.md) · [Upstream notices](../../NOTICE.md)

## Directional and contact shadows

A thin glasses rim exposed a mismatch between the screen-space contact estimate and the directional map. At ground level, a distant moving caster could also brighten the edge of a nearby shadow. The port addresses those cases at different stages of the renderer.

| Change | Behavior and review points |
| --- | --- |
| Sun and moon filtering | P3 evaluates percentage-closer soft shadows against light-space depth. Blocker-to-receiver separation and the light’s angular diameter set the penumbra. A stable light-space sampling pattern keeps the filter consistent as the camera moves. Filter radius is bounded by 0.25 m and 256 local texels. Disabling softness selects the fixed filter. The beta’s separate screen-space directional soft-shadow compute and blur passes and their WGSL file are removed. |
| Near and distant blockers | When layered filtering is active, casters render into overlapping local and remote light-depth ranges. Remote tiles have half the local width and height and occupy the atlas’s second mip level. Both ranges are sampled along corresponding directions across the light disk. Their combined visibility is capped by local visibility, so a crossing remote penumbra can darken an already-shadowed point or leave it unchanged. Casters within the same range can still share a blocker estimate. |
| Cascade transitions | A dedicated detail map and three progressively coarser levels cover the requested distance. The four-level path spaces the coarser ends geometrically, blends adjacent maps across the last 15% of each interval, and samples both maps at the same offset receiver. It fades the outermost shadow over the last 10% of its range. The fine-map overlap can retain supported texels beyond the nominal transition. Invalid map samples fall back to a coarser map that covers the receiver. |
| Coarse presets and thin geometry | Four-level edge smoothing accounts for map resolution and cascade level. On 512 and 1024 px maps the middle and far levels use a narrower filter to preserve thin silhouettes. Graded half-coverage caster dithering is anchored to the projected world origin. The close filter uses local depth and receiver slope to distinguish thin casters from detached edges. System avatars and worn attachments carry a receiver flag for the avatar-specific bias and slope treatment. Linked children update when their owner changes, including on transform-only updates. |
| Camera-close map | When the graded detail fit and adapter permit it, a 1024 × 1024 depth map covers a 50 cm square perpendicular to the light, about 0.49 mm per texel. The crop snaps to the parent light grid and preserves the parent depth projection. Near a face it uses the crop for local detail and the larger map for distant blockers. Its influence fades across the outer 10 cm of the crop and as camera-to-receiver distance grows from 32 to 45 cm. Changing optical zoom alone does not change that distance. |
| Contact composition | The final geometric visibility is the minimum of mapped visibility and depth-filtered contact visibility. An overlapping estimate cannot double-darken the receiver or cause the mapped shadow to brighten. The beta intensity slider scales the artistic result after geometric composition. The classic-sky lighting path scales the shadow contribution to its normal-dependent lighting. Objects, terrain, water and snow retain intensity handling. |
| Bend contact controls | Bend remains the stock contact tracer. Custom offers original, 120, 240 or 480 px search lengths and a bounded adaptive reach in metres. Extending the search preserves a short hard-contact span. The first hard samples keep their existing depth test. Farther along a ray, the tracer uses surface-depth continuity and actual ray crossings to limit how much hidden thickness it assumes. It tapers its contribution near the end of a truncated screen-space ray. Controls cover modeled occluder depth, optional adaptive ray depth, and optional same-surface repair of small gaps in the contact mask. Screen-space coverage depends on visible depth. |
| Alternative contact tracer | An experimental view-space ray method uses physical reach, depth tolerance, and a configurable sample budget. It shares the depth-aware mask filter with Bend. Sparse sampling can miss thin geometry, and rays reaching the screen edge lose coverage. |
| Render passes, hardware and cost | CPU and GPU culling admit casters through the fine map’s independent light box. Planar reflections use the main camera’s fitted directional maps. Adapter limits gate the four-level and camera-close bindings. A remote mip adds 25% to the base directional atlas allocation and replays caster draws. The close map adds 4 MiB of depth storage and an additional caster pass when active. Photo quality budgeting accounts for those allocations. These are allocation and pass costs. Desktop frame-time measurements remain open. |

[Directional filter and composition](../../crates/aurora-render/src/shaders/common.wgsl) · [Projection, allocation and passes](../../crates/aurora-render/src/renderer.rs) · [Bend and view-space dispatch](../../crates/aurora-render/src/contact_shadows.rs) · [Avatar and attachment receiver flags](../../crates/aurora-viewer/src/scene/mod.rs)

## Shadow quality, persistence and diagnostics

Each enabled stock tier selects four-level directional maps, mild edge smoothing, soft sun and moon shadows, Bend contact shadows and shadow-casting projectors. Map size, reach, tracer length and projector budget are selected together. The slightly larger light diameter on Low and Medium softens steps at coarse resolutions.

| Stock tier | Map side | Balanced directional reach | Finest map reach | Light diameter | Bend search in window | Projector limit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Low | 1024 px | 48 m | 2 m | 0.65° | 120 px | 1 |
| Medium | 2048 px | 96 m | 2 m | 0.65° | 120 px | 2 |
| High | 4096 px | 144 m | 3 m | 0.53° | 120 px | 4 |
| Ultra | 4096 px | 192 m | 2 m | 0.53° | 480 px | 8 |

- **Custom and Advanced.** Each tier keeps its own Custom settings. A user can switch to a stock tier and return to the saved edits. The reset control restores only the selected tier. Closing Advanced leaves the values active. Custom includes Near, Balanced and Far coverage, soft shadows, angular diameter, and projector controls. Turning off softness disables the angle slider in place and preserves its value. Advanced exposes three- and four-level layouts, independent map resolution and reach, first-cascade coverage or fine-map range, edge smoothing, contact settings and diagnostics.
- **Upgrades and named profiles.** Recognized stock Aurora 0.9.12 and beta settings adopt the corresponding P3 tier. Distinct manual controls import into Custom. The master shadow switch and global intensity survive migration. An explicitly saved stock-valued beta softness switch cannot be distinguished from an untouched switch. Persistent source diameter is bounded to 0.10°–2°. Older P3 settings and named graphics profiles migrate their per-tier history. Inactive edits survive switching tiers or disabling shadows.
- **Photos and projectors.** Phototools can override shadow softness and intensity while the photo controls are open, including the full beta photo-softness range. Closing the override restores the exact world angle and Custom map size. Tiled captures sample the same light-space directional maps. Stock projectors use P3’s five-tap filter. With soft sun shadows active, `AURORA_SOFT_PROJECTORS=1` enables the beta’s main-view projector soft filter. A blocker search that misses a thin edge now falls back to the five-tap estimate. The Graphics panel reports the measured number of active shadow-casting projectors, allocated capacity and map dimensions.
- **Diagnostics.** A single selector exposes shadow-map, effective-contact and combined visibility, cascade coverage, filtered contact occlusion, Bend candidates and failures, and view-ray outcomes. Method-specific modes identify missing prerequisites. Graphics can display measured cascade ends and texel widths, plus near-detail GPU timing when available. A shared Rust control definition supplies the WGSL feature bits and diagnostic values. Crash summaries use the active P3 directional-softness state.

[Tier definitions and migration](../../crates/aurora-viewer/src/settings/shadow_profile.rs) · [Named profiles](../../crates/aurora-viewer/src/graphics_presets/saved.rs) · [Graphics controls](../../crates/aurora-viewer/src/ui/options/pages/graphics.rs) · [Photo light controls](../../crates/aurora-viewer/src/snapshot/light.rs) · [Renderer diagnostic definitions](../../crates/aurora-render/src/render_controls.rs)

## Materials

| Control | Initial setting | Rendering change |
| --- | --- | --- |
| Local PBR thin-surface lighting | Off. The default scope selects a single material UUID, initially empty. | Matches world glTF faces. A second scope previews all double-sided, alpha-masked PBR faces. A magenta highlight can show which faces matched. The viewer stores the first eight bytes of the material UUID in spare per-face GPU fields, so UUIDs with the same prefix can also match. HUDs are excluded. |
| Authored occlusion floor | 0.35, adjustable from 0 to 0.8 | For matching faces, takes the greater of the material’s sampled occlusion and the selected floor. This acts on the red channel of the occlusion/roughness/metallic texture. |
| Backlight transmission | 0.40, adjustable from 0 to 1.5 | Adds a tint from the base color when the directional light strikes the back of a matched surface. The geometric normal sets the response and a directional shadow lookup bounds the light. Both the classic and modern PBR lighting branches support the approximation. |
| Harmonize legacy lighting | Off. Three individually selectable corrections start enabled under the master switch. | In modern environment lighting, legacy sunlight can use a Lambertian diffuse scale of 0.96/π, diffuse ambient occlusion can move 18% toward full visibility, and point lights and projectors can use a diffuse-only contribution in the GGX path so the authored legacy Blinn–Phong highlight is applied once. |

Material selection, diagnostic highlighting, and lighting adjustments are viewer preferences. They operate on local draw records and shading. Materials in inventory and in-world objects require no edits. To compare, select one leaf material and inspect its backlit face next to a different material. For the legacy controls, compare a glossy legacy face under a point light or projector and toggle each correction individually. [Per-face material keys](../../crates/aurora-viewer/src/scene/mod.rs) · [Material shading](../../crates/aurora-render/src/shaders/object.wgsl) · [Legacy lighting terms](../../crates/aurora-render/src/shaders/common.wgsl)

## Ambient occlusion

**Thin-object AO compensation** exposes XeGTAO’s existing thin-occluder parameter as a live slider from 0 to 1.5. Its default of zero preserves the beta’s GTAO response. Higher values can reduce excessive darkening around narrow silhouettes such as foliage, wires, or hair. Check an adjacent solid crevice when adjusting this control, since its occlusion can also weaken. This is a scene-wide GTAO setting. The PBR occlusion floor above applies to the authored material and can be evaluated independently. [GTAO uniform](../../crates/aurora-render/src/gtao.rs) · [Graphics controls](../../crates/aurora-viewer/src/ui/options/pages/graphics.rs)

## Depth of field

- **Include alpha-blended surfaces** gives the world and Photo lenses the visible depth of blended geometry, including detached strands. The same 10% effective opacity cutoff as FSR applies to texture alpha multiplied by face alpha. Gaps retain the background depth. Each pixel still represents a single surface depth.
- **AA and resolution.** None, FXAA, SMAA, MSAA and TAA can use a dedicated single-sample lens depth target. FSR 3.1 reuses its blended depth at native size or its display-resolution reconstruction when upscaling. The lens can request its own pass if FSR's blended motion pass does not run. The separate lens surface keeps ambient occlusion, temporal motion and selection independent of this preference.
- **Smooth near-focus blur** introduces a gentler start to foreground blur as the subject leaves the focus plane. It adjusts the half-resolution foreground gather and coverage ramps and adds a conditional full-resolution, eight-neighbour filter weighted by circle of confusion. Strength ranges from 0 to 1 with a saved default of 0.85.
- **Show DoF focus zones** displays near blur in red, far blur in blue and focused areas in grey. It also works in Photo mode alongside the beta’s existing focus controls. Blended surface depth defaults on for new or missing preferences and retains saved choices. Smoother blur and focus zones start off. [Viewer lens settings](../../crates/aurora-viewer/src/dof.rs) · [Depth selection](../../crates/aurora-render/src/renderer.rs) · [Blur and composite shader](../../crates/aurora-render/src/shaders/dof.wgsl)

## Persistence, localization and documentation

- General viewer settings retain the thin-material target, gains and highlight, legacy toggles, GTAO compensation and DoF controls. Named graphics profiles capture the legacy harmonization settings and the DoF settings block alongside their shadow controls. Thin-material targeting and GTAO compensation remain general preferences when switching named profiles. [Viewer settings](../../crates/aurora-viewer/src/settings.rs) · [Named-profile field list](../../crates/aurora-viewer/src/graphics_presets/saved.rs)
- English, French and Japanese include labels, options, help and diagnostics for the shadow, material, GTAO and depth-of-field controls. The shared square icon button now exposes its button role, label and enabled state to the UI accessibility layer. [English](../../crates/aurora-i18n/locales/en.ftl) · [French](../../crates/aurora-i18n/locales/fr.ftl) · [Japanese](../../crates/aurora-i18n/locales/ja.ftl) · [Button metadata](../../crates/aurora-viewer/src/ui/widgets.rs)
- The repository README links the P3 feature guide. The architecture overview describes the layered directional and contact path. The testing guide updates the shadow override and projector behavior. The wiki generator registers the settings and validation pages under **For testers**. [Feature guide](README.md) · [Scene validation](VALIDATION.md) · [Architecture](../../ARCHITECTURE.md) · [Testing](../TESTING.md) · [Wiki generator](../../scripts/build-wiki.ps1)

## Interpreting the recorded results

The original beta port passed all 119 focused viewer checks. The full renderer suite on SwiftShader returned 618 passed, two failed and 37 ignored. The two failures, a legacy color comparison and water coverage, reproduced on unmodified `231c88a3` with the same adapter. A separate run of the normally ignored shadow visibility and contact-detail regression passed.

Five synthetic close-up scenes reproduced every recorded P3 v2.4.3 directional sample (184,168 samples). The single-rim Low case produced zero false second bands in 285 scans. In the admitted 500 m moving-caster case, the nearby shadow core showed zero measured brightening. The original camera-motion tests also recorded weaker results at a cropped screen edge and on a fast pullback. The surface lighting, animation, and performance of a real avatar still need a desktop comparison. [Reference measurements and test scenes](VALIDATION.md)
