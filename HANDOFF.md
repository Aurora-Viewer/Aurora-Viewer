# Handoff: empty Trash with RLVa enabled (#2255)

Branch: `fix/trash-rlva-2255`

The branch merges current `origin/main`; the PR's task entry and the newer main entries are all retained in `TASKS.md`.

## Validate

With RLVa enabled, empty Trash when the Trash contains no locked worn item and confirm it completes. Also verify a locked worn item still blocks the destructive action with the existing message. No grid connection was made in this worktree.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
