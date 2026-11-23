# Implementation status — Aurora Extension Platform v1

Current state: **Draft POC in progress; not production-ready.** The current local increment adds bounded extension-owned key/value storage behind a separate `extension.storage` grant; the fixed `extension.full_access` bundle still covers only the four chat/UI capabilities and excludes storage. No unrestricted native code or Viewer internals are exposed. Focused verification on the current source: `cargo test --workspace --locked full_access -- --nocapture` passed 6 tests (API coverage, runtime manifest/dispatch/revocation, viewer persistence/isolation/catalog/panel behavior), and `cargo test --workspace --locked ui::options::pages::extensions -- --nocapture` passed the permission-catalog test (1); both commands had nonzero matched test counts. Previously recorded focused workspace verification also passed 54 options tests, 1 wrapping-tabs test and 1 theme-guard test; these are separate historical results, not added to the current run totals. Current `cargo fmt --all -- --check` and strict workspace Clippy (`cargo clippy --workspace --all-targets --locked -- -D warnings`) both passed. This branch contains a generic `.ext` SDK/runtime/manager and neutral offline panel/notification sample only. No product-specific extension source or assets are included. The workspace test run is red: 11,501 passed / 24 failed / 63 ignored across 44 summaries. All 24 failing tests reproduce individually on clean base `77c7553`; no branch-only failures were found. `scripts/check.ps1` passed parsing, shell hook, formatting and strict Clippy; the v0.3.0 package build/validation and Wasmi storage round-trip passed. Manual viewer UI smoke and cross-platform CI remain outstanding.

Current extension-storage verification: workspace `storage` filter passed 22 tests, 0 failed, with 2 matching tests ignored across 5 binaries; `full_access` passed 8/8 and locale-catalog parity passed 1/1. The exact persistent lifecycle and Viewer round-trip filters each passed 1 test; lifecycle coverage includes permission revocation/regrant, per-extension isolation, update retention, corruption/quota rejection and uninstall cleanup. The v0.3.0 Wasm sample packed, validated and ran through a temporary Wasmi host harness, persisting toggle state across manager reopen and respecting safe mode. Workspace formatting, strict Clippy and diff checks pass. Manual Viewer UI smoke, cross-platform CI and independent security review remain.

| Stage | State | Evidence | Remaining |
|---|---|---|---|
| 0 Reconnaissance / ADR | Recorded | `ADR-001.md`; existing bounded ZIP validation and Wasmi interpreter/no-WASI design reviewed | Cross-platform targets, CVE response and dependency footprint require maintainer review |
| 1 Package / API / SDK | Implemented in POC | Versioned API/SDK, bounded `.ext` ZIP/manifest validation, deterministic pack/validate CLI, optional Ed25519 signature envelope | Manual viewer UI smoke and cross-platform CI remain |
| 2 Runtime / lifecycle | Implemented in POC | Import-free Wasmi execution with fuel/memory limits; explicit grants/revocation, persistent lifecycle, quarantine and safe mode; opaque account/grid-scoped store; bounded `extension.storage` (32 entries, 64-byte ASCII keys, 1 KiB values, 8 KiB persisted quota); `extension.full_access` cannot read/write storage; corrupt/oversized storage is rejected without rewriting; over-quota writes preserve previous values; tested permission revoke/regrant, per-extension isolation, persistence across update and uninstall cleanup; existing registry/trust corruption, rollback and same-panel-refresh regressions pass | Migration and recovery when reopening persisted state itself fails; independent security review |
| 3 Viewer manager | Implemented in POC; UI smoke pending | Preferences manager, package review, permission approval, lifecycle controls, safe mode and host-rendered panels/actions; Viewer round-trip verifies explicit storage grant, persisted toggle state after reopen and host filtering of `SetStorage` | Manual install/review/grant and rendered-UI validation on this branch |
| 4 Hardening / cross-platform | In progress | Strict Clippy and focused extension tests pass; full workspace run and clean-base comparison are recorded above | Track the reproduced upstream failures; add cross-platform CI and independent security review |
| 5 Upstream contribution | Draft POC in progress | Separate draft contribution for the generic SDK/manager and neutral sample | Human UI smoke, cross-platform CI and maintainer review remain |

## Baseline and branch

- Base: official `Aurora-Viewer/Aurora-Viewer` `main`, commit `77c7553bb1ab10d647ef7f78184b4b3e3c7dc113`.
- POC branch: `feat/extension-sdk-manager-poc`.
- Scope: generic `.ext` SDK/runtime/manager and a neutral panel/notification sample. Product-specific plugins/assets and detached-window capabilities are outside this contribution.
- No extension-specific issue was selected; this remains an in-progress proof of concept for maintainer review.

## Historical verification log (previous local worktree)

The entries below describe earlier local validation and are not evidence of UI validation on this draft branch; the current branch's results are stated at the top of this document.

- `cargo test -p aurora-extension-runtime --locked`: 24 passed, 0 failed. This includes package bounds, generic API execution, consent/revocation, in-memory and persistent update reapproval, quarantine, and persistent restart/uninstall tests.
- `cargo test -p aurora-i18n --locked`: 26 passed, 0 failed across the library and catalog tests, including the new English/French/Japanese extension permission explanations.
- `cargo test -p aurora-viewer --locked extension_chat_tests -- --nocapture`: 3 passed, 0 failed. The tests cover accepted-other-resident input sanitization, event/annotation binding, and clearing stale chat targets.
- `cargo check -p aurora-viewer --locked`: passed after the latest update, host-action, and localized permission UI changes.
- The first viewer test run reported 9,068 passed, 26 failed, 22 ignored. Two failures came from this extension page: the theme guard rejected direct egui widgets and the icon test rejected the missing `puzzle-filled` asset. The page now uses shared Aurora widgets and the existing embedded `package` icon; both targeted tests pass individually. The remaining 24 failures are not yet classified.
- The current Phase 2 `scripts/check.ps1` passed PowerShell parsing, agent hook, formatting and strict Clippy, then failed at tests. Summing the 26 Cargo test-result summaries gives 10,877 passed, 21 failed and 63 ignored; the failing `aurora-viewer --bin aurora-viewer` target accounts for 9,077 / 21 / 22 (86.70 s). The serial workspace run produced the same aggregate totals; its viewer target reported 9,077 / 21 / 22 (509.70 s). Programmatic comparison found identical default/serial named failure sets (21/21). On clean base `8a9d00e`, `scripts/check.ps1` stopped at the base-only renderer test failure; a separate `cargo test --workspace --locked --no-fail-fast` completed all targets with 10,864 passed, 22 failed and 63 ignored across 37 summaries. Exact-filter reruns of all 21 viewer failures using the compiled test executables, one at a time with `RUST_TEST_THREADS=1`, failed individually on both worktrees; the water-depth test passed individually on the feature branch and failed on base. Programmatic comparison: all 21 current failures reproduce on base, zero current-only, and the base-only failure is `shader_tests::water_mesh_depth_clamp_preserves_coverage_and_occlusion_on_gpu`, corrected by earlier local test-oracle commit `67bd023d0`. Thus the prescribed check remains red but no branch-only failing test was found.
- Previous failure inventory and assertion summaries (the current Phase 2 default run reproduced the 21 entries below except for the earlier default-only agent timing failure):
  - `agent::tests::a_stop_eases_out_without_a_step_or_a_pull_back` — earlier timing assertion: `fps 60, release 0: accel max 29.2 m/s², rise 0.011 m/s, pull back 0.014 m`; it passed when rerun alone both with and without `RUST_TEST_THREADS=1` and did not fail in the current run.
  - `build::ui::compact::tests::construction_hides_forbidden_geometry_and_feature_parameters` — `Feature value 4.731, legacy=false`; expected true, got false.
  - `build::ui::compact::tests::layout_tests::settings_follow_the_free_side_and_bottom_of_the_inspector` — `settings.right() <= main.left()` failed.
  - `build::ui::compact::tests::typed_value_over_the_other_mixed_texture_fields_reaches_every_face` — `legacy=true Glossiness: no field beside Połysk`.
  - `ui::appearance::tests::clipboard_keys_take_the_selected_rows_of_outfits_and_worn` — `a copy of the selected items`.
  - `ui::appearance::tests::gallery_double_click_reopens_the_folder_without_wearing_it` — expected 1, got 0.
  - `ui::appearance::tests::review_2376_worn_show_original_dispatches_the_original_without_changing_the_outfit` — `Pokaż oryginał`.
  - `ui::appearance::tests::the_outfit_list_keeps_its_height_while_it_lays_out_what_is_in_sight` — `Kurtka skórzana moved`; expected `[43.0, 3944.0]`, got `[19.0, 3754.0]`.
  - `ui::appearance::tests::wearable_rows_precede_attachments_in_outfits_worn_and_editor` — `Alfa below attachment in view 0`.
  - `ui::blacklist::sound_tests::blacklist_copying_multiple_categories_copies_the_owner_uuid_once` — incomplete category label `Dźwięki zrezzowanych obiektów`.
  - `ui::blacklist::sound_tests::blacklist_sound_category_row_removes_only_the_clicked_category` — missing control `Dźwięki zrezzowanych obiektów`.
  - `ui::land::tests::dates_like_the_french_skin` — got `So 18 cze 2022 11:19:10`, expected `Sat 18 Jun 2022 11:19:10`.
  - `ui::land::tests::money_groups_thousands` — got `88 000`, expected `88,000`.
  - `ui::people::radar::tests::rlva_shownearby_empties_the_list_and_showminimap_hides_its_map` — localized `radar-empty` assertion failed.
  - `ui::place_details::tests::landmark_dates_in_slt` — got `Wt 02 kwi 2024 07:42:28`, expected `Tue 02 Apr 2024 07:42:28`.
  - `ui::scripts::tests::search_moves_between_left_floating_and_inline_without_losing_fields_or_focus` — got `id_D493`, expected `id_71CF`.
  - `ui::sound_explorer::tests::sound_explorer_native_widgets_select_pause_filter_and_dispatch_all_actions` — got `None`, expected `Some(Blacklist(4))`.
  - `world::groups::tests::a_failed_group_message_says_what_to_do` — denial message did not start with `world-groups-action-denied` translation.
  - `world::rlva::privacy::tests::an_rlvanonym_link_shows_the_stand_in_of_the_name_once_it_is_known` — failed at `Osobnik`.
  - `world::rlva::privacy::tests::rlva_anonyms_are_the_legacy_list_picked_by_the_sum_of_the_name` — anonymous display-name assertion failed.
  - `world::rlva::privacy::tests::rlva_hidden_names_use_the_stand_in_of_their_legacy_name_everywhere` — hidden-name display assertion failed.
  - `world::rlva::privacy::tests::the_name_of_an_object_that_speaks_loses_the_names_and_keeps_the_places` — failed at `Tamten agent`.
- Earlier local offline smoke reached Preferences › Extensions, but it was performed in a different worktree and is not evidence of UI validation on this draft branch.
- An earlier generic `.ext` review displayed the manifest identity and requested permissions; the package was left uninstalled during that review-only step.
- Current-branch neutral fixture build and in-memory runtime validation are summarized above; the temporary harness was removed after the run and the binary is not committed.
- A signed neutral sample was previously validated locally. Its artifact, public key and fingerprint are not part of this contribution.
- Phase 1 targeted notification regressions: `RUST_TEST_THREADS=1 cargo test --workspace --locked extension_host::tests -- --nocapture` passed both tests. The earlier Phase 1 Clippy result is historical; strict workspace Clippy was rerun on the current Phase 2 source and passed (see above).
- Phase 2 focused workspace tests (serial, each filtered by test name, no `--exact`): `extension_host::tests::persistent_package_and_permission_grant_are_isolated_between_accounts` 1 passed; `extension_host::tests::account_scopes_are_opaque_and_isolated_by_grid_and_account` 1 passed; `tests::signed_package_verifies_and_tampered_content_is_blocked_before_wasm_validation` 1 passed; `tests::unsigned_install_requires_ack_and_local_key_trust_persists_and_revokes` 1 passed. The account test installed/enabled the generic fixture under synthetic account A, confirmed account B saw no package/grants/panels and stale effects were discarded, then restored A's state. The trust test confirmed explicit revocation clears prior unverified acknowledgement and requires fresh review.
- Earlier offline lifecycle checks exercised the neutral sample's panel action, in-view notification, safe mode and manager controls. They were not performed on this draft branch; its manual UI smoke remains pending.
- Clean-base exact-filter comparisons on `77c7553` reproduced all 24 failing tests; no branch-only failures were observed.
- The POC does not modify renderer or shader code. Its validation did not launch the viewer or change user settings.

## Historical acceptance notes (previous local worktree)

These notes describe earlier implementation gaps and are retained as historical context, not as current-branch test results.

Earlier local notes documented additional manager hardening and cross-platform review needs. They are retained as historical context; current scope and validation for this draft are stated above.
