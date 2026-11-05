# #2396 — Process pose invites during full-frame photo mode

Branch: `fix/poser-invite-photo-2396`.

The per-frame `tick_poser_consent` now drains queued poser actions and updates consent state before UI rendering. It runs ahead of the full-frame photo branch, so an Ask request is no longer stuck waiting for the normal UI. The photo branch also draws the incoming consent prompt before returning. The offline `AURORA_DEMO_POSER=photo-request` scenario queues an outgoing invite and displays an incoming consent prompt over the photo.

## Validation

Commands to run from this worktree:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```

Results:

- `cargo fmt --all --check` — passed.
- `cargo test -p aurora-viewer queued_pose_invite_is_transmitted_during_full_frame_photo_mode` — passed (1 test).
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored. The failures are GPU/render tests under this Linux RADV environment: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `clippy::manual_range_contains` warning in `crates/aurora-assets/src/anim.rs:211`; Clippy did not reach a clean workspace result.

The focused regression test is `app::poser::tests::queued_pose_invite_is_transmitted_during_full_frame_photo_mode`.

## Human validation

1. Run the offline poser demo with `AURORA_DEMO=1 AURORA_DEMO_POSER=photo-request`.
2. At full-frame photo mode, confirm the queued pose invite is processed and an incoming consent prompt appears over the photo.
3. Answer the prompt and confirm its response is processed without leaving photo mode.

No grid connection was made. Real bridge transport remains a two-client in-world check.
