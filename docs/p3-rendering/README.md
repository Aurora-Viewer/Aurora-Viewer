# P3 rendering changes

The P3 v2.4.3 rendering changes were originally tested on Aurora `v0.9.13-beta.20261121.2` (`231c88a3`). The directional maps, caster overlap, screen-space contact shadows, close-up crop and stock shadow tiers follow P3. Aurora's separate shadow intensity, photo controls and main-view projector filtering remain available. The branch includes Materials, Ambient Occlusion and Depth of Field.

[Complete change register and review notes](CHANGES.md) · [Validation and reproduction](VALIDATION.md)

## Integration with Aurora ray tracing

The contribution integrates upstream `1ab3bac1` while retaining the shadow-map recipes described below. Where ray tracing covers a surface, Aurora's traced visibility, glass tint and global-illumination openness remain active. Other surfaces retain the P3 directional-map fallback. Harmonize and local thin-surface lighting retain the traced light's tint.

**Deluxe (RT)** combines High's other graphics settings with ray-traced effects and the Ultra fallback shadow tier. Choosing High explicitly restores the High tier. Saved settings use schema 48 so older official schema-47 writers preserve the added shadow-profile fields. Migrations distinguish historical Deluxe, current Aurora profiles, and profiles saved by the reviewed rendering client.

## Shadow controls

In Graphics, choose a shadow quality level. With ray tracing off, each enabled stock level uses graded directional maps, Bend screen-space contact shadows, and **Soft sun and moon shadows**. The last option gives a wider penumbra when a caster is farther from its receiving surface. **Light angular diameter** controls the apparent width of the emitter. The slightly wider Low and Medium defaults soften visible steps at those resolutions.

| Stock level | Map setting | Balanced sun shadow distance | Light angular diameter | Shadow-casting projector limit |
| --- | ---: | ---: | ---: | ---: |
| Low | 1024 | 48 m | 0.65° | 1 |
| Medium | 2048 | 96 m | 0.65° | 2 |
| High | 4096 | 144 m | 0.53° | 4 |
| Ultra | 4096 | 192 m | 0.53° | 8 |

The projector limit is a budget for simultaneous shadow casters. Available atlas space can reduce the effective count. **Custom** stores edits separately for each tier. Turning it off restores the tier’s stock values and retains edits for the next time Custom is enabled. Turning off **Soft sun and moon shadows** disables the angle control in place. Closing **Advanced** keeps its values active. The reset arrow beside Custom restores the selected tier. Named graphics profiles carry the shadow settings with them. [Tier recipes and persistence](../../crates/aurora-viewer/src/settings/shadow_profile.rs)

## How the directional maps and contact shadows interact

Raster contact shadows include blended texels from 50% coverage. A glasses rim at 75% opacity contributes its own depth and can receive a contact shadow. Ray tracing and ambient occlusion use their separate solid-surface depth rule; the lens independently includes strands from 10% effective opacity. Enabling ray tracing does not replace the Bend or lens depth buffers.

- **Different caster heights.** Layered soft shadows draw nearby and distant blockers into separate light-depth ranges. The remote layer uses tiles at half the local tile width and height. The filter samples both layers in corresponding directions across the sun disk and bounds the result by the local visibility. This addresses a moving overhead caster causing an already-shadowed point to brighten. Casters falling within one depth range still share a blocker estimate.
- **Very close receivers.** An optional camera-centred 1024 × 1024 depth map covers a 50 cm square perpendicular to the light. Its 0.49 mm texels can better align a thin rim’s directional shadow with its screen-space contact shadow. The crop snaps to the parent light-space grid and retains its depth projection. The shader still looks up distant blockers in the larger map. The effect fades across the outer 10 cm of the crop and over 32–45 cm of camera-to-surface distance. Optical zoom alone leaves that distance unchanged.
- **Composition and cost.** The darker of directional and effective contact visibility determines direct-light visibility. The extra distant mip adds 25% of the base atlas allocation when active and replays directional caster draws. The close map adds a 4 MiB depth allocation and one caster pass on eligible frames. It requires a graded layout, useful finer coverage and adequate device limits. The renderer can use the ordinary detail map when those conditions fail. These figures describe allocations and passes. Desktop GPU timings remain to be measured. [Directional lookup](../../crates/aurora-render/src/shaders/common.wgsl) · [Projection and map allocation](../../crates/aurora-render/src/renderer.rs)

## Compare material lighting, ambient occlusion and focus

Use a fixed camera, environment, exposure and graphics profile for the before and after views. Change one control at a time. The material controls change what this viewer renders locally. They do not write to an object or material asset.

### Harmonize legacy lighting

In **Preferences → Graphics → Image**, use a modern EEP sky and frame a glossy legacy object beside a PBR object. Include a point light or projector if checking local highlights. **Harmonize legacy lighting** defaults off. When enabled, its three controls initially all apply. Toggle each individually.

| Control | What to inspect |
| --- | --- |
| **Normalize sunlight diffuse** | Look at a directly sunlit legacy face. The new diffuse scale is 0.96/π. A nearby PBR face provides a reference for brightness. |
| **Soften diffuse ambient occlusion** | Look at the legacy face in a corner or recess. Its diffuse ambient term moves 18% toward unobstructed illumination. |
| **Avoid duplicate local highlights** | Move the camera across a local light's highlight on a glossy legacy object. The renderer retains the authored Blinn–Phong response and removes the additional GGX contribution there. |

### Local PBR thin-surface lighting

In **Graphics → Light**, enable **Enable local PBR thin-surface lighting**. It defaults off. Leave **Target scope** on **Specific material UUID** and enter the asset UUID for a leaf or fabric glTF material from the object editor. The default empty field matches no material. **Highlight matched materials** tints matches magenta. Use this to check which faces will change, then turn it off to judge their lighting. **Preview masked + double-sided PBR** expands the effect to all qualifying world faces. HUDs are excluded.

Hold the material in front of the sun. Set **Backlight transmission** to zero, then restore its default of **0.40** and compare the backlit color. To inspect authored occlusion, compare **Minimum PBR material AO** at zero and its default of **0.35** on a face whose occlusion texture contains dark areas. A neighbouring face with a different material provides a control. The sliders span 0–1.5 and 0–0.8 respectively. Targeting uses the first eight bytes of the UUID on the GPU, so materials with the same prefix can also match.

### Thin-object ambient occlusion

In **Graphics → Light → Ambient occlusion**, select Low or High and set **Occlusion method** to **GTAO**. Frame narrow hair, foliage or a wire next to a solid crevice. Compare **Thin-object AO compensation** at its default of **0** and at **0.5**. The higher setting can reduce dark halos around the thin geometry. Check whether it also weakens the real shadow in the crevice. This scene-wide slider ranges from 0 to 1.5 and is independent of the occlusion floor on a selected PBR material.

### Depth of field on blended hair

In **Graphics → Image → Depth of field**, enable the world lens, lock focus on a subject with alpha-blended hair, and use a distant, textured background. **Include alpha-blended surfaces** defaults on for new or missing preferences. An existing profile retains its saved value. The option also applies when only the Photo lens is active.

1. Compare the option off and on at the same pose. At 10% effective opacity or above, a strand in front of the background contributes its depth. Transparent gaps retain the background depth. Texture and face alpha both count. Overlapping translucent layers still share one depth per pixel.
2. Keep focus and blur fixed and cycle None, FXAA, SMAA, TAA, the available MSAA levels, and FSR 3.1 Native AA and upscaling. Inspect the outer strands and the gaps separately. **Show DoF focus zones** marks near blur red, far blur blue and the focus region grey. Disable the overlay to compare the final image.
3. To evaluate **Smooth near-focus blur**, move a foreground object slowly through the near edge of focus. The option defaults off. When enabled its strength defaults to **0.85** on a 0–1 scale. Check the onset of foreground blur at the same lens settings.

Named graphics profiles store the legacy harmonization and DoF controls. The selected PBR material and GTAO compensation are general viewer preferences and persist across profile changes.

[Graphics controls](../../crates/aurora-viewer/src/ui/options/pages/graphics.rs) · [GPU focus fixture](qa/DOF-HAIR.md) · [Shadow replay](qa/bridge-pullback/README.md) · [Validation and reproduction](VALIDATION.md)
