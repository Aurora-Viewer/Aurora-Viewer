# #2409 — Preserve 2K glTF material maps on upload

Branch: `fix/material-map-2k-2409`.

Material entries now use a 2048×2048 maximum for their maps, matching the Legacy Viewer material editor's `MAX_IMAGE_SIZE_DEFAULT`. The preference that limits ordinary texture uploads to 1024 does not silently reduce material maps. The regression test loads a 2048×1024 map and checks that it remains that size under both preference states.

## Validation

Commands to run from this worktree:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```

Results:

- `cargo fmt --all --check` — passed.
- `cargo test -p aurora-viewer texture_size_limit_defaults_off_and_new_rows_capture_the_latest_persisted_choice` — passed (1 test).
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored. The failures are GPU/render tests under this Linux RADV environment: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `clippy::manual_range_contains` warning in `crates/aurora-assets/src/anim.rs:211`; Clippy did not reach a clean workspace result.

No grid connection was made.

## Human validation

1. Import a glTF with a material map larger than 1024, such as a 2048×1024 base color map.
2. Confirm the preview retains the 2K map dimensions and proceed through upload.
3. Inspect the uploaded map and confirm its dimensions are 2048×1024.
4. Repeat with the “Limit to 1024” texture preference enabled, if available; material maps should still upload at 2K.
