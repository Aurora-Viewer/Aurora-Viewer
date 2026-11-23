# `.ext` package format (v1 draft)

An `.ext` file is a ZIP archive, not a native library. It contains exactly one `manifest.json`, the `module.wasm` entry module, zero or more non-executable files under `assets/`, and an optional root-level `signature.json` envelope.

```text
ExtensionFixture.ext
├── manifest.json
├── module.wasm
├── signature.json                 # optional Ed25519 signature envelope
└── assets/icon.png                 # optional
```

Example manifest:

```json
{
  "manifest_version": 1,
  "id": "dev.example.extension-fixture",
  "name": "Extension Fixture",
  "version": "0.1.0",
  "api": ">=1.0.0, <2.0.0",
  "entry": "module.wasm",
  "permissions": [],
  "network_domains": [],
  "publisher": "Example developer"
}
```

The current validator accepts only manifest version 1, semantic versions and Rust `semver::VersionReq` API ranges compatible with the viewer's supplied API version (for example `>=1.0.0, <2.0.0`). See `manifest.schema.json` for the structural schema. Unknown manifest fields and permissions fail closed. The initial permission allowlist is `chat.nearby.read`, `ui.chat.annotate`, `ui.panel.register` and `ui.notification.show`; package metadata with non-empty `network_domains` is rejected until the network broker and consent UX exist. The parser does not grant permissions.

Archive limits in this draft: 32 MiB compressed input, 64 MiB expanded payload, 32 MiB per file, 128 ZIP entries, eight path components and a maximum 100:1 expansion ratio. The bounded end-of-central-directory preflight rejects over-limit counts and Zip64/multi-disk archives before the ZIP library constructs its entry index. Absolute paths, traversal, non-ASCII paths, backslashes, control characters, Windows reserved names (including space-padded device-name stems), duplicate/case-colliding paths, file/directory conflicts, symlinks and special files are rejected. Entries are validated and read into bounded memory; this crate does not extract paths to disk.

The content SHA-256 covers all package files sorted by UTF-8 archive path, excluding `signature.json`. For each file the validator hashes the little-endian 64-bit path byte length, path bytes, little-endian 64-bit content length, then content bytes. The optional signature envelope is JSON with `schema_version: 1`, `algorithm: "ed25519"`, the SHA-256 public-key fingerprint in `key_id`, and lowercase-hex `public_key` and `signature` fields. The Ed25519 signature covers a domain-separated, length-prefixed extension ID, version, API range and content digest. This authenticates control of a key, not a legal identity or safe behavior. A missing/unknown key is unverified; a malformed or invalid signature is blocked. There is no central registration authority.

The validator preflights and bounds the ZIP, validates the manifest and optional signature, then compiles/instantiates `module.wasm` with Wasmi 2.0.2. Invalid signatures are rejected before guest execution. Imports, WASI and start functions are disabled; package validation checks the v1 `memory`, `alloc` and `handle` export signatures without invoking guest events. Each `GuestModule::handle_event` call uses the versioned JSON protocol described in `abi-v1.md`, validates the response against its request, and uses a fresh fuel-limited store. Per-instance limits are 16 MiB linear memory, 1,024 table elements and a 1 MiB value stack, with strict module compilation limits. Phase 2 adds local key trust and explicit unverified/identity-change consent; its UI and persistence tests are still in progress. Guest state does not persist between calls. Only generic fixtures belong in the Aurora contribution; production plugins are separate projects.
