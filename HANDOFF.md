# #2405 — Follow camera pitch while flying in mouselook

Branch: `fix/mouselook-flight-pitch-2405`.

For unseated mouselook, `AgentControls.body_rotation` now includes yaw and camera pitch, matching the Legacy Viewer frame rotation sent to the simulator. `AgentState::body_rotation()` remains yaw-only for drawing the avatar. The local flight stand-in uses the same pitch for forward/back movement, while walking remains level. Seated controls keep their frame rotation unchanged.

## Validation

Commands to run from this worktree:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```

Results:

- `cargo fmt --all --check` — passed.
- Both focused tests listed below — passed.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored. The failures are GPU/render tests under this Linux RADV environment: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `clippy::manual_range_contains` warning in `crates/aurora-assets/src/anim.rs:211`; Clippy did not reach a clean workspace result.

Focused tests:

- `camera::tests::mouselook_controls_send_pitch_without_tilting_the_drawn_body_or_seated_avatar`
- `agent::tests::local_mouselook_flight_follows_pitch_while_walking_stays_level`

## Human validation

1. In-world, enter mouselook and fly forward while looking up; confirm you climb.
2. Look down and fly forward; confirm you dive.
3. Walk in mouselook while looking up or down; confirm walking stays level.
4. Sit in an object and verify its seated mouselook orientation is unchanged.

No grid connection was made. Human in-world validation is pending.
