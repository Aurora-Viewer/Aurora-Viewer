# Extension capability security matrix

This matrix separates implemented API v1 behavior from proposed capability families. A proposed capability is not available to a guest until its permission, limits, broker, UI consent and tests exist. This contribution contains only a neutral test fixture; production extensions are separate projects.

## Implemented API v1

| Capability | Permission / grant rule | Threat boundary and host contract | Input/output limits | Evidence and tests |
|---|---|---|---|---|
| Accepted nearby chat | `chat.nearby.read`; host checks the grant before constructing the event | Only already-accepted, sanitized nearby resident text. No IM, group chat, UUID, object, blocked or hidden message. Guest cannot initiate chat. | Speaker 256 UTF-8 bytes; text 4 KiB; total JSON envelope 16 KiB; host-generated opaque message ID. | API validation tests; viewer privacy/event-boundary tests. |
| Chat annotation | `ui.chat.annotate`; checked again at host action application | Plain-text annotation only, bound to the triggering message ID; never edits the original message or supplies markup. | 2 KiB UTF-8 text; 16 actions maximum per response. | Correlation/mismatch and permission-denial tests. |
| In-view notifications | `ui.notification.show`; host checks grant before creating a card | Aurora renders plain text; Do Not Disturb and toast preferences apply. No guest markup or OS desktop-notification request. | Title 128 UTF-8 bytes; body 2 KiB; response envelope/action caps apply. | WASM-to-notification regression covers the card action and permission denial; manual UI smoke on this draft branch remains pending. |
| Declarative panel | `ui.panel.register`; registration accepted only on Initialize and checked against the grant | Host owns rendering and controls. Guest supplies labels/buttons/toggles only; no HTML, JavaScript, layout callbacks, native handles or executable UI. | 16 controls per panel; UI strings 2 KiB; panel/control IDs 64 ASCII bytes; envelope and action caps apply. | Panel shape, limits, duplicate IDs and lifecycle tests; current viewer UI smoke pending. |
| Package execution | No runtime permission; package grant set is separate | No WASI/imports, filesystem, sockets, secrets or viewer pointers. Fresh Wasmi store/memory/fuel per invocation; trap and capability violations quarantine. | Archive 32 MiB compressed, 64 MiB expanded, 32 MiB per member, 128 ZIP entries, 100:1 ratio; guest memory 16 MiB, table 1,024, stack 1 MiB, fuel 1,000,000 per call. | Adversarial ZIP/WASM/ABI tests; focused viewer extension-host regressions. |

## Additional capability boundary

This proof of concept exposes only the implemented v1 capabilities above. Any future capability requires a separate threat model, explicit user consent, a bounded host broker and adversarial tests before guest code can use it.

## Trust and update states

- **Verified Publisher:** signature validates over the canonical package contents and the signing key is locally trusted. The displayed publisher label is a user-assigned key label, not proof of legal identity.
- **Unverified Plugin:** unsigned or signed by an unknown key. Installation may be offered only with explicit acknowledgement; sandbox and requested-permission consent still apply.
- **Blocked Plugin:** invalid signature, failed package/security validation, post-install content mismatch, or an explicit local block rule when block rules are implemented. Validation failures are rejected before execution; there is not yet a user-managed explicit blocklist or a recoverable installed-item badge for every restore-time mismatch.
- **Permissions Changed:** update adds a permission beyond the previously approved grant set. Keep the old package active or the extension disabled until the user reviews and explicitly approves the change; never inherit new grants.
- **Built-in owner trust (future, not implemented):** any pinned trust root must be a specific owner-provided public-key fingerprint. The eventual trust pin must bind to a specific owner-provided public key/fingerprint; neither the publisher label nor namespace alone grants trust. Never embed a private key. Define rotation and revocation; the generated demo signer is test-only.

The package-content SHA-256 is integrity metadata, not publisher authentication. This POC includes optional Ed25519 signing/verification and user-managed local key trust; a valid signature alone does not imply trust. Current workspace validation: 11,501 passed / 24 failed / 63 ignored across 44 summaries. All 24 failing tests reproduce individually on clean upstream `main` at `77c7553`, with no branch-only failures; strict Clippy passed. Interactive UI smoke and cross-platform CI remain pending.
