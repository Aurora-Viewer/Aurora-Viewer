# Handoff: camera filter search focus (#2044)

Branch: `fix/camera-search-2044`

The branch now includes current `origin/main`; its existing camera-search task entry and both newer main task entries remain in `TASKS.md`.

## Validate

Open the camera window, click Filters search, type and edit a query. Confirm the query filters the controls and the avatar does not move while typing; the other camera controls retain their keyboard shortcuts.

No grid connection was made. This worktree was not connected to a grid.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
