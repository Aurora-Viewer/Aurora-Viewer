# Handoff: god-mode texture UUID visibility (PR #2226)

Branch: `feat/opensim-god-mode`

The shared inventory UUID visibility rule now accepts the current god-level state. It is used by texture preview Copy UUID and material editor map Copy UUID. The texture preview's Save As action still requires full permissions. A regression verifies a god can reveal a UUID with no inventory item while non-god rules remain permission-gated.

## Human validation in-world

On an OpenSim test region, grant the account god level and open a restricted texture in the texture preview and a restricted material map in the material editor; verify Copy UUID is available. Repeat without god level and verify it stays disabled. Confirm Save As remains disabled for the restricted texture in both cases. Do not use a production grid.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo test -p aurora-viewer an_asset_uuid_shows_for_full_permissions_or_a_built_in_texture --locked` — passed, 1 test.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `manual_range_contains` lint at `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the workspace run. The failures are GPU/RADV-sensitive: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` (depth delta) and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` (`0.39999998` versus `0.4`).

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```
