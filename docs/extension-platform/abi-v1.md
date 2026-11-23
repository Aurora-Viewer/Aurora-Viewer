# Extension event ABI v1

This is the versioned, import-free message boundary for generic extensions. The host exposes no WASI, filesystem, network, secret or viewer-object imports. A guest cannot call back into Aurora; it returns declarative actions which the host validates and authorizes before applying.

## WASM exports

A guest module exports:

- `memory`: one linear memory.
- `alloc(i32 byte_count) -> i32 address`: a bounded allocator. The host independently verifies returned ranges.
- `handle(i32 input_address, i32 input_length, i32 output_address, i32 output_capacity) -> i32 output_length`: one JSON request/response call.

`aurora-extension-sdk` provides the `ExtensionHandler` trait, typed API re-exports and `export_extension!(HandlerType)` macro. A guest receives one JSON `ExtensionRequest` and returns one JSON `ExtensionResponse` as defined in `aurora-extension-api`. Both carry `api_version = 1` and a host-generated `request_id`; the response must echo that ID.

## Events and actions

API v1 events are `initialize`, `nearby_chat`, `panel_action` and `shutdown`. Nearby-chat events carry only a host message ID, sanitized display name and accepted nearby text; there is no account ID, UUID, IM, group-chat, object, hidden or blocked-message event. The host must check `chat.nearby.read` before constructing a `nearby_chat` event.

Responses may request `annotate_chat`, `show_notification`, `register_panel` or `set_storage`. `show_notification` creates a host-rendered Aurora in-view card from plain text; Do Not Disturb and the viewer's toast preferences apply, and no OS desktop notification is required or sent. Panels consist only of host-rendered labels, toggles and buttons; HTML, JavaScript and arbitrary layout are not part of v1. The runtime checks protocol shape and message correlation. The manager must separately check the matching user grant (`ui.chat.annotate`, `ui.notification.show`, `ui.panel.register` or `extension.storage`) before applying an action. `network.fetch` and domain access are not implemented or accepted.

## Limits and isolation

The JSON envelope is capped at 16 KiB, nearby text at 4 KiB, UI strings at 2 KiB, actions at 16 per response, panel controls at 16 per panel, and panel identifiers at 64 ASCII bytes. `extension.storage` exposes at most 32 key/value entries, with 64-byte ASCII keys, 1 KiB values and an 8 KiB serialized store per extension. The host rejects unknown fields, unsupported API versions, mismatched request IDs, annotations for other messages, duplicate panel/control IDs, duplicate storage keys, invalid UTF-8 and control characters.

The host allocates separate input and output ranges, checks overflow, bounds and overlap, then calls `handle`. A Wasmi store receives a fresh 1,000,000 fuel budget per invocation; memory is capped at 16 MiB, tables at 1,024 elements and the value stack at 1 MiB. Imports, start functions and malformed export signatures are rejected. Traps and fuel exhaustion return errors. Guest memory is fresh per event, so extensions must not rely on module globals persisting. When `extension.storage` is granted, the host includes a scoped `storage` snapshot in `ExtensionRequest` and persists validated `set_storage` actions; when the permission is not granted, the optional field is omitted. `extension.full_access` does not include storage.

The protocol types and SDK are generic framework code. Any example kept in this repository is a minimal identity/test fixture, not a production plugin. No product plugin is included in the upstream contribution.
