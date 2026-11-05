# Handoff: Show original on the Worn tab (#2345)

Branch: `fix/worn-original-2345`

The branch includes current `origin/main`; its PR task entry and newer main TASKS entries are retained.

## Validate

Wear an item with a linked inventory original, open the Worn tab's context menu, and choose Show Original. Confirm it selects/opens the source item. Confirm the action is absent for entries without a linked original. No grid connection was made in this worktree.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
