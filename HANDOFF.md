# Handoff: build menu shape icon view (#2303)

Branch: `fix/build-shape-icons-2303`

The branch now contains current `origin/main`; the shape-icon task entry and newer main entries are retained in `TASKS.md`.

## Validate

Open Build, select each primitive shape, and inspect the shape icons at the default and resized window sizes. Confirm their outlines match the shape preview and do not appear skewed or shifted. No grid connection was made in this worktree.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
