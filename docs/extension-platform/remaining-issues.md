# Aurora Extension Platform — remaining work

Status: **draft proof of concept, in progress; not production-ready.** The contribution is limited to the generic `.ext` SDK/runtime/manager and a neutral offline sample. No product-specific extension source or assets are included.

## Locally implemented

- Optional Ed25519 package signatures with a bounded envelope; the signature binds the package ID, version, API range and canonical content digest. Unsigned and valid-but-unknown keys are distinct from malformed/invalid signatures.
- CLI key generation, optional package signing and signature validation.
- User-managed publisher-key trust records, fingerprint review, verified/unverified state, explicit unverified/key-change acknowledgement, and added-permission review. A publisher label is only a local alias; trust does not establish legal identity or safety.
- Extension package, grants, trusted keys and lifecycle state are scoped to a stable opaque hash of grid login URI plus agent UUID. Raw identifiers are not passed to extensions or used as the directory name. The worker clears pending reviews, panels and queued effects and hides old-scope state while switching; it drops the old manager without dispatching a guest `Shutdown` across accounts.
- Native Aurora Preferences manager, local `.ext` file review, lifecycle controls, host-rendered panels and in-view notification actions remain the existing platform path; no embedded web manager or production plugin was added.
- `extension.storage` is a separate, explicit grant (never implied by `extension.full_access`), scoped per extension and active account/grid. It is bounded to 32 keys, 64-byte ASCII keys, 1 KiB values and an 8 KiB persisted file; no filesystem or network access is exposed. Revocation disables the extension and hides retained data; uninstall removes it.

## Incomplete or not yet validated

- The final workspace-filtered storage suite passed 22 tests with 0 failures and 2 matching ignored tests; the `full_access` filter passed 8/8, the English/French/Japanese catalogue parity test passed 1/1, and the exact storage lifecycle and Viewer round-trip filters each passed 1/1. These cover storage revocation/regrant, isolation between extension IDs, update persistence, limits, corruption handling and uninstall cleanup.
- The trusted-key import/trust/revoke and signer-change UI have not been smoke-tested interactively in the current viewer. Cryptographic invalid-signature/tamper tests and earlier CLI signing smoke are not substitutes for UI validation.
- Built-in publisher trust (not implemented as a hard-coded root): any trusted key must be explicitly reviewed and bound to a specific public-key fingerprint. Labels and package-ID namespaces alone do not authenticate a publisher; never embed a private key, and define key rotation and revocation before adding an owner-pinned trust root.
- Scope boundary: this POC includes the generic `.ext` SDK/runtime/manager and neutral panel/notification sample only. Detached-window support and product-specific plugins are outside this contribution.
- The neutral v0.3.0 Wasm fixture was built, packed and CLI-validated; a temporary Wasmi manager harness confirmed storage-backed toggle persistence across manager reopen and safe-mode denial. Manual Viewer UI smoke and cross-platform CI remain pending.
- No Linux/macOS/ARM CI result is available for these changes.
- Legacy root-level extension data is intentionally left untouched and is not automatically migrated into account scopes. No data was deleted; users may need to reinstall packages per account. A deliberate migration/review path is still needed before release.
- Explicit key distrust clears earlier unverified acknowledgements and disables packages signed by that key; the persistent trust/revocation test passed and verifies fresh acknowledgement is required. The UI flow still needs offline smoke validation.
- The trust UI does not yet provide a user-managed explicit blocklist or a recoverable blocked-item status for every restore-time integrity mismatch. Failed-update package rollback/retention also needs adversarial tests.
- The native manager still lacks search, filters, categories, optional extension-directory/catalogue discovery, richer changelog/developer tools and the complete update workflow requested for Phase 2.
- API v1 remains limited to its documented, permission-checked capabilities. Any additional capability is outside this POC and requires its own threat model, explicit consent, bounded host broker and tests.

## Verification gates before upstream readiness

1. Strict workspace Clippy passed; the full workspace test run is red at 11,501 passed / 24 failed / 63 ignored. Preserve that distinction and do not report the suite as passing.
2. Clean-base exact-filter comparison: all 24 failing tests reproduce on clean upstream `main` at `77c7553`; no branch-only failures were observed.
3. Manual Viewer install/review/grant/UI smoke for the current draft branch remains pending. The neutral fixture has not been installed interactively in the Viewer and no human UI permission review has been performed during this validation.
4. Remaining focused hardening: blocked rules, restore-time tamper handling, key rotation/revocation and failed-update preservation. Account-scoped worker commands are verified by a passing regression covering stale chat and panel actions after an account switch.
5. Obtain cross-platform CI and human review; this PR remains a draft until the repository owner decides it is ready. Do not merge without maintainer approval.
