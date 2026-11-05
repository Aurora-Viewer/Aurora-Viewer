# #2385 — Replace script dialogs from the same object and channel

Branch: `fix/script-dialog-replace-2385`.

The Legacy Viewer default `ScriptDialogLimitations = 1` replaces a script dialog on the same object and channel. Aurora now removes an existing `Data::Dialog` or `Data::TextBox` for that pair before adding a new prompt. Replaced prompts are discarded without a reply, matching `LLScriptFloaterManager::onAddNotification` in `indra/newview/llscriptfloater.cpp`. Because replacement happens before insertion, the total interactive script-dialog count does not dip and the close sound should not fire.

A regression test verifies same-object/channel replacement across dialog and text-box types, that different objects and channels remain, and that the incoming event emits no reply.

## Validation

Checks run from the worktree:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace
```

Results:

- `cargo fmt --all --check` — passed.
- `cargo test --workspace script_dialogs_replace_only_the_same_object_and_channel_without_reply` — passed (1 test).
- `cargo test --workspace` — 352 passed, 2 failed, 32 ignored. The failures are existing GPU/render tests under this Linux RADV environment: `water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu` and `pbr_terrain_samples_all_layers_maps_and_triplanar_normals_on_gpu`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — blocked by the existing `clippy::manual_range_contains` warning at `crates/aurora-assets/src/anim.rs:211`; Clippy did not reach a clean workspace result.

`scripts/check.ps1` is unavailable in this Linux environment.

## In-world validation

1. Put the issue’s four-line `llDialog` script in a prim.
2. Click the prim several times. Confirm only one dialog for that object/channel remains and it displays the latest prompt.
3. Verify the replacement does not send a reply to the old dialog and does not play a close sound between prompts.
4. Open dialogs from another object and from another channel; they should remain stacked independently.
5. Repeat with `llTextBox` and with a dialog replacing a text box on the same object/channel.

No grid connection was made. Human in-world validation is pending before PR creation.
