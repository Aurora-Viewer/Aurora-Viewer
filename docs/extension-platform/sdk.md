# Extension SDK quickstart (v1 draft)

The upstream framework includes two generic crates:

- `aurora-extension-api`: versioned JSON request/response types, event/action enums and permission names.
- `aurora-extension-sdk`: `ExtensionHandler`, request validation and `export_extension!(HandlerType)` to export the guest `alloc` and `handle` ABI.

Production extensions are separate projects. This document describes only the generic developer interface; no product plugin is included in the framework contribution.

## Minimal test handler

```rust
use aurora_extension_sdk::{ExtensionAction, ExtensionEvent, ExtensionHandler};

#[derive(Default)]
struct MinimalFixture;

impl ExtensionHandler for MinimalFixture {
    fn on_event(&mut self, event: &ExtensionEvent) -> Vec<ExtensionAction> {
        match event {
            ExtensionEvent::Initialize { .. } => vec![ExtensionAction::ShowNotification {
                title: "Fixture ready".into(),
                body: "This is a generic API test fixture.".into(),
            }],
            _ => Vec::new(),
        }
    }
}

aurora_extension_sdk::export_extension!(MinimalFixture);
```

The host sends an `ExtensionRequest` JSON envelope and the SDK returns a correlated `ExtensionResponse`. The host still enforces the manifest's requested and user-granted permissions; SDK validation is not a grant. Each event is handled with fresh guest state in the current runtime prototype. Guest globals do not persist; extensions that request and receive `extension.storage` can read the bounded `request.storage` snapshot and return `ExtensionAction::SetStorage` changes instead.

## Compile and package

The repository includes a neutral offline fixture at `crates/aurora-extension-sdk/examples/demo_extension.rs` with its manifest beside it. It registers a host-rendered panel, persists the toggle through the separately granted `extension.storage` capability, restores it on initialization, refreshes only its own panel in response to its `PanelAction`, and requests a host-rendered in-view notification when its button is clicked. It reads no chat and has no network, filesystem or secret capability.

Install the Rust `wasm32-unknown-unknown` target if needed, then build the fixture using the workspace's default development profile:

```powershell
rustup target add wasm32-unknown-unknown
cargo build -p aurora-extension-sdk --example demo_extension --target wasm32-unknown-unknown
cargo run -p aurora-extension-cli --bin aurora-ext -- pack --manifest crates/aurora-extension-sdk/examples/demo_extension.manifest.json --module target/wasm32-unknown-unknown/debug/examples/demo_extension.wasm --output .scratch/aurora-sandbox-demo.ext
cargo run -p aurora-extension-cli --bin aurora-ext -- validate .scratch/aurora-sandbox-demo.ext --api-version 1.0.0
```

To sign a release build, generate a local Ed25519 key pair and pass the private-key file to `pack`:

```powershell
cargo run -p aurora-extension-cli --bin aurora-ext -- keygen --private-key <secure-private.pk8> --public-key .scratch/publisher.pub.json
cargo run -p aurora-extension-cli --bin aurora-ext -- pack --manifest crates/aurora-extension-sdk/examples/demo_extension.manifest.json --module target/wasm32-unknown-unknown/debug/examples/demo_extension.wasm --output .scratch/aurora-sandbox-demo-signed.ext --signing-key <secure-private.pk8>
cargo run -p aurora-extension-cli --bin aurora-ext -- validate .scratch/aurora-sandbox-demo-signed.ext --api-version 1.0.0
```

Never commit the private key or place it in a public package/repository. The signature proves control of the key and package integrity; it does not establish a legal identity or safety. `validate` verifies the cryptographic signature but cannot decide whether the current user's local trust store trusts the key. The signed `.ext` contains only the public key, fingerprint, and signature. In the viewer, import the generated public-key JSON through Preferences › Extensions › Trusted Publishers, compare the displayed fingerprint, and explicitly trust it before treating that signer as verified; without that local trust decision, the valid signature remains unverified.

In an offline demo viewer (`AURORA_DEMO=1`), open Preferences › Extensions and install the generated unsigned `.ext`. Review the displayed identity and permissions, explicitly acknowledge the unverified publisher, grant `ui.panel.register`, `ui.notification.show` and `extension.storage`, and enable the fixture. The toggle sends a `PanelAction`; its new value is persisted in the extension's local account/grid-scoped storage and the guest's refreshed declaration updates that panel. The button exercises the notification path. The fixture is a test aid, not a production extension.

The packer sorts archive members, writes fixed timestamps/modes, rejects unsafe assets, validates the generated archive before writing, and refuses to overwrite an existing package. The viewer manager is integrated locally with explicit permission review, install/update/enable/disable/uninstall, revocation, safe mode and quarantine. See `IMPLEMENTATION_STATUS.md` for current validation status.

Supported manifest permissions are `chat.nearby.read`, `ui.chat.annotate`, `ui.notification.show`, `ui.panel.register` and the separately granted `extension.storage`. `extension.full_access` bundles only the four chat/UI permissions; it does not include storage. Storage is limited to 32 keys and 8 KiB per extension under the active account/grid; it is not shared across extensions or sent over the network. Revoking storage access disables the extension and hides the retained data until it is explicitly regranted; uninstalling the extension removes the data. Network access, filesystem access, secrets, IM/group-chat capture and native libraries are not available in v1. The publisher field is self-asserted; the package hash is an integrity identifier, not a signature.

## Current limitations

This draft POC is not production-ready. The workspace test run is red: 11,501 passed / 24 failed / 63 ignored across 44 summaries. All 24 failures reproduce individually on clean upstream `main` at `77c7553`, with no branch-only failures; formatting and strict Clippy passed. The neutral fixture was built, packed, validated and executed through the in-memory manager; manual install/review/grant UI smoke on this branch and cross-platform CI remain pending. There is no public extension store or production plugin.
