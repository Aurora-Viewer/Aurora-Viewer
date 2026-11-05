# Handoff: Ctrl+Alt keyboard layout characters (#2048)

Branch: `fix/iso-at-input-2048`

The branch includes current `origin/main`; the PR task entry and newer main task entries are retained in `TASKS.md`.

## Validate on Windows

Use an ISO keyboard layout where `@` is typed with Ctrl+Alt+2. In a text field, type `@` and confirm it appears, the viewer does not treat it as a control shortcut, and other Ctrl/Alt shortcuts still work when no printable character is produced. Repeat with a second layout if available. No grid connection was made in this worktree.

## Checks

- `cargo fmt --all --check` — passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by existing `manual_range_contains` in `crates/aurora-assets/src/anim.rs:211`.
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored before the render crate stopped the run. The failures are the known GPU/RADV-sensitive water depth and PBR terrain float tests.
