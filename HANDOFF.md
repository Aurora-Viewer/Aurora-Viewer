# Handoff: name+link inventory search (#1854)

Branch: `fix/inventory-link-search-1854`

The branch merges current `origin/main`; the issue's task entry and current main entries are all kept in `TASKS.md`.

## Validate

In inventory, search for a linked item using its displayed name plus `link` (for example `Object link`). Confirm the link is returned alongside normal name matches. Confirm ordinary searches still work. No grid connection was made in this worktree.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
