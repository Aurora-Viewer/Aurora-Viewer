#3365 RLVa empty camera texture is the flat grey

## What changed and why

An empty `@camtextures` or `@setcam_textures` substituted the plywood texture (`DEFAULT_TEXTURE`), so the world went wood-grain. The Legacy Viewer uses `IMG_DEFAULT` for that restriction and paints it locally as solid 0x7f grey.

`camera_texture()` now returns `aurora_prim::te::IMG_DEFAULT` (`d2114404-dd59-4a4d-8e6c-49359e91bbf0`) when the value is empty, so `@getcam_textures` reports that id. An explicit uuid is still parsed and fetched. `=y` still clears the modifier.

`scene/textures.rs` treats that id as built in, the same way it treats the blank and transparent textures, and binds `aurora_render::textures::MISSING` (the existing 0x7f grey). `acquire` returns that slot and creates no entry and no fetch. `refresh`, `face_problem`, `is_settled`, the preview and bake images, and the UI image path treat it as resident. Particles already acquire whatever `camera_texture()` returns, so they get the grey without a separate change. World faces still replace only objects that are not worn and are not plants, so avatars and attachments keep their textures.

The plywood id is unchanged for new prims and the build tools.

## Legacy Viewer sources

- `indra/llcommon/indra_constants.cpp` (`IMG_DEFAULT`, also declared in `indra_constants.h`)
- `indra/newview/llviewertexture.cpp` (`LLViewerTextureManager::init` writes a 128×128 solid 0x7f image into `IMG_DEFAULT`)
- `indra/newview/rlvhelper.cpp` (`addModifier` for `RLV_BHVR_SETCAM_TEXTURES` defaults to `IMG_DEFAULT`; `camtextures` is the synonym)
- `indra/newview/rlvhandler.cpp` (`RlvBehaviourModifierHandler<RLV_MODIFIER_SETCAM_TEXTURE>::onValueChange` applies the id as the object diffuse default and clears it when the restriction lifts)

No visible difference from that behaviour was chosen. Aurora binds the existing grey slot instead of allocating a new 128×128 image.

## Tests

Compiled in this worktree before the behaviour change (`/tmp/3365-failfirst.log`, EXIT 101):

- `rlva_render_aliases_release_without_mutating_saved_settings_and_touch_measures_the_surface` left `Some(89556747-24cb-43ed-920b-47caed15465f)` (plywood) and right `Some(d2114404-dd59-4a4d-8e6c-49359e91bbf0)`.
- `only_streamed_textures_are_refreshed` failed at `assert!(!refresh(IMG_DEFAULT))` because a refresh still asked for a download.

The grey-pixel, settled, and preview assertions sit in that same texture test and did not run on the failing pass, because the refresh assertion stopped it. `reserved_slot(IMG_DEFAULT) == MISSING` was added with the fix. `acquire` itself is not called from the test: it needs a renderer. `acquire` returns `reserved_slot` for this id and does not create an entry.

After the fix (`/tmp/3365-retest.log`, EXIT 0) both tests passed, including the grey pixels `[0x7f, 0x7f, 0x7f, 0xff]`, `is_settled`, the preview image, and the `MISSING` slot.

## Checks

Toolchain 1.99.1. Base `983c9b164`. `cfg(windows)` was not touched.

- `cargo +1.99.1 fmt --all --check`: EXIT 0
- `cargo +1.99.1 clippy --workspace --all-targets --locked -- -D warnings`: EXIT 101, exactly the 17 accepted Linux dead-code warnings (`write_plain`, `preserve`, `parse_driver_date`, `luid_text`, `adapter_used`, `path_ids`, `fold_serial`, `usable_uuid`, `pick_windows`, `arrived`, `ABSOLUTE`, `VIRTUAL_DESKTOP`, `PIXELS`, `RANGE`, `Packet`, `Absolute`, `packet`). None are in files this branch changes.
- `cargo +1.99.1 test --workspace --locked --no-fail-fast`: EXIT 101. The only failures are `shader_tests::water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` and `ui::fonts::tests::what_inter_lacks_comes_from_the_same_faces_as_with_noto`. `terrain_tests::pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` passed. `slurl_handoff::tests::closing_viewer_refuses_links` passed. aurora-render: 448 passed, 1 failed, 34 ignored. aurora-viewer: 7417 passed, 1 failed, 16 ignored. The two tests above passed inside that run.
- `cargo +1.99.1 build -p aurora-viewer --locked`: EXIT 0. Binary `target/debug/aurora-viewer` is 660M. The same 17 dead-code warnings, and no others.

## What to test by hand

Offline, no grid:

- `AURORA_DEMO_RLVA=textures-grey`. After the restriction applies, world surfaces and particles are flat grey. The avatar and its attachments keep their textures. `AURORA_DEMO_RLVA=textures-grey-released` restores the surfaces in the same run.
- `AURORA_DEMO_RLVA=textures` still substitutes the demo gradient, not the grey.

On a grid where RLVa is enabled (log in yourself):

- `@camtextures=n` or `@setcam_textures=n`: the world is plain grey, avatars and attachments keep their textures.
- `@camtextures:<some texture uuid>=n`: that texture is used instead of the grey.
- `=y` restores the real textures.
