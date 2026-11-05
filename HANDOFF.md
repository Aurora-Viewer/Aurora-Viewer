# Handoff: texture picker shared UUID permission rule (#2381)

Branch: `fix/texture-uuid-permissions-1943`

The picker now calls `world::inventory::actions::asset_uuid_shown`, the rule already used by material UUID display. Its existing local asset, Blank/Transparent/blank-material and baked texture exceptions remain. The inventory-link regression exercises a link to an original item and permission mask changes.

No new in-world check is needed for this follow-up: the PR's original behavior was already validated by a human on the grid. Do not connect to a grid for code validation.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo test -p aurora-viewer texture_uuid_uses_the_shared_asset_rule_and_resolves_links --locked` — passed, 1 test.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `manual_range_contains` lint at `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the workspace run. The failures are GPU/RADV-sensitive: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` (depth delta) and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu` (`0.39999998` versus `0.4`).

Both newer main TASKS entries were retained along with the existing #1943 entry during the merge.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```
