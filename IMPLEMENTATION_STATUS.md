# Implementation status — Aurora Extension Platform v1

Current milestone: bounded extension-owned key/value storage. `extension.storage` is explicitly granted, separately scoped to the extension and active account/grid, and excluded from the fixed `extension.full_access` bundle. API compatibility/limits, persistent grant-gated storage and the Viewer host round-trip are verified. Draft POC, not production-ready.

Verified on the current `feat/extension-sdk-manager-poc` worktree:
- `cargo test --workspace --locked full_access -- --nocapture`: passed 6 matching tests, 0 failed. API coverage (1), runtime manifest and dispatch/revocation (2), viewer persistence/isolation/catalog/panel paths (3).
- `cargo test --workspace --locked ui::options::pages::extensions -- --nocapture`: passed 1 matching permission-catalog test, 0 failed. The catalog test also matched the previous filter; do not add both command totals as unique test cases.
- `cargo test --workspace --locked persistent_manager_rejects_duplicate_registry_entries_without_rewriting_them -- --nocapture`: passed 1 matching runtime test, 0 failed; duplicate IDs were rejected and registry bytes stayed unchanged.
- `cargo test --workspace --locked persistent_manager_rejects_oversized_registry_before_parsing_or_rewriting_it -- --nocapture`: passed 1 matching runtime test, 0 failed; the over-1-MiB input was rejected before parsing and remained byte-identical.
- `cargo test --workspace --locked persistent_manager_rejects_excess_registry_entries_before_loading_packages -- --nocapture`: passed 1 matching runtime test, 0 failed; 33 entries were rejected before package loading and registry bytes stayed unchanged.
- `cargo fmt --all -- --check` and strict workspace Clippy passed after the count-limit regression was added.
- `cargo test --workspace --locked persistent_manager_rejects -- --nocapture`: passed 8 matching runtime tests, 0 failed, covering registry corruption/record validation, size/count limits and trusted-publisher count/size limits.
- `cargo test --workspace --locked persistent_update_rolls_back_memory_and_new_package_when_registry_commit_fails -- --nocapture`: passed 1 matching runtime test, 0 failed; the old installed version/state remained active and the unreferenced new package was removed.
- `cargo test --workspace --locked persistent_manager_rejects_excess_trusted_publishers_without_rewriting_them -- --nocapture`: passed 1 matching runtime test, 0 failed; 257 trust records were refused before key validation and bytes remained unchanged.
- `cargo fmt --all -- --check` and strict workspace Clippy passed after the trust-store count test was added.

Historical separately recorded focused checks: 54 options tests, 1 wrapping-tabs test and 1 theme-guard test passed. The full workspace remains red (11,501 passed / 24 failed / 63 ignored; all 24 reproduced individually on clean base `77c7553`, no branch-only failures), as detailed in `docs/extension-platform/IMPLEMENTATION_STATUS.md`.

- `cargo test --workspace --locked persistent_manager_rejects_oversized_trust_registry_without_rewriting_it -- --nocapture`: passed 1 matching runtime test, 0 failed; 256 KiB + 1 byte rejected and file preserved.
- `cargo test --workspace --locked persistent_manager_rejects_invalid_trusted_publisher_label_without_rewriting_it -- --nocapture`: passed 1 matching runtime test, 0 failed; the control-character label was rejected and the file remained unchanged.
- `cargo test --workspace --locked persistent_manager_rejects -- --nocapture`: passed 10 matching runtime tests, 0 failed, including duplicate trusted-publisher keys and invalid trust labels.
- `cargo test --workspace --locked persistent_manager_rejects_duplicate_trusted_publisher_keys_without_rewriting_them -- --nocapture`: passed 1 matching runtime test, 0 failed; duplicate valid keys were rejected and bytes remained unchanged.
- `cargo test --workspace --locked panel_action_can_refresh_only_the_panel_that_emitted_it -- --nocapture`: passed 1 API test, 0 failed; same-panel refresh is accepted and cross-panel refresh is rejected.
- `cargo test --workspace --locked panel_click_runs_wasm_with_full_access_and_shows_an_in_view_card_while_active -- --nocapture`: passed 1 viewer integration test, 0 failed; guest panel refresh updated host toggle state while existing notification path remained valid.
- `cargo build --locked -p aurora-extension-sdk --example demo_extension --target wasm32-unknown-unknown`: passed; the updated neutral toggle sample compiled as Wasm.
- `cargo run --locked -p aurora-extension-cli --bin aurora-ext -- pack --manifest crates/aurora-extension-sdk/examples/demo_extension.manifest.json --module target/wasm32-unknown-unknown/debug/examples/demo_extension.wasm --output .scratch/aurora-demo-panel-refresh.ext`: passed; produced the unsigned neutral sample package.
- `cargo run --locked -p aurora-extension-cli --bin aurora-ext -- validate .scratch/aurora-demo-panel-refresh.ext --api-version 1.0.0`: passed; package validated with only `ui.panel.register` and `ui.notification.show` requested.
- `cargo test --workspace --locked persistent_update_reopens_committed_state_when_rollback_save_fails -- --nocapture`: passed 1 runtime test; an injected rollback-registry write failure reopens the durably committed update and removes the now-unreferenced old package.
- `cargo test --workspace --locked persistent_update_ -- --nocapture`: passed 3 runtime tests, 0 failed, including prior rollback and successful update coverage.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `git diff --check`: passed after panel-refresh and rollback-recovery changes.
- `cargo test --locked -p aurora-extension-api -- --nocapture`: passed 8 API tests, 0 failed; added optional-storage backward compatibility, explicit permission exclusion from full access and storage bounds.
- `cargo test --locked -p aurora-extension-runtime persistent_extension_storage_is_grant_gated_and_removed_on_uninstall -- --nocapture`: passed 1 runtime test; storage persists across reopen and is removed on uninstall.
- `cargo test --locked -p aurora-extension-runtime storage_quota_rejection_preserves_previous_values -- --nocapture`: passed 1 runtime test; over-quota writes preserve the prior serialized state.
- The earlier package-only runtime suite passed 47/47 before the final storage lifecycle assertions; final workspace-filtered results are recorded below.
- `cargo test --locked -p aurora-extension-api storage_rejects_invalid_keys_values_duplicate_keys_and_excess_entries -- --nocapture`: passed 1 API test; exact maximum key/value lengths are accepted and over-limit request/action values are rejected.
- `cargo check --locked -p aurora-extension-runtime -p aurora-extension-sdk --all-targets`: passed.
- `cargo check --locked -p aurora-viewer --all-targets`: passed after wiring the storage snapshot through the viewer host.
- `cargo test --locked -p aurora-viewer panel_action_persists_only_explicitly_granted_extension_storage -- --nocapture`: passed 1 test after clearing the queued Initialize batch; it verifies storage restoration after reopen and host filtering of `SetStorage`.
- `cargo fmt --all -- --check`: passed after formatting the storage lifecycle test. `cargo clippy --workspace --all-targets --locked -- -D warnings && git diff --check`: passed against the current worktree; only Git line-ending warnings remained.
- `cargo test --quiet --workspace --locked storage -- --nocapture`: passed 22 tests, 0 failed, with 2 matching tests ignored across 5 test binaries; includes storage bounds, corrupted/quota rejection, full-access output denial, lifecycle/revoke/regrant, isolation/update/uninstall and Viewer storage host coverage.
- `cargo test --quiet --workspace --locked full_access -- --nocapture`: passed 8 tests, 0 failed.
- `cargo test --quiet --workspace --locked bundled_catalogs_use_english_keys_and_variables -- --nocapture`: passed 1 catalog test, 0 failed; English/French/Japanese keys and variables match.
- Exact workspace filters `persistent_extension_storage_is_grant_gated_and_removed_on_uninstall` and `panel_action_persists_only_explicitly_granted_extension_storage` each passed 1 test, confirming the storage lifecycle test and Viewer round-trip in their intended targets.
- Wasm demo build, `aurora-ext pack`, and `aurora-ext validate` passed for v0.3.0; package requests `ui.panel.register`, `ui.notification.show`, and `extension.storage`.
- `CARGO_TARGET_DIR=F:/extaurorawork/work/extension-sdk-manager-poc/target cargo run --offline --manifest-path .scratch/storage-smoke/Cargo.toml -- .scratch/aurora-demo-extension-storage.ext`: passed; the packaged Wasm ran through Wasmi, persisted its toggle across manager reopen, and was blocked by safe mode. Harness and package remain in ignored `.scratch`.
