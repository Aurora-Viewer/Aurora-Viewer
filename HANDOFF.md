# Issue #2048 — Preserve Windows Ctrl Alt layout characters in text fields

Branch: `fix/iso-at-input-2048`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2048

## Change and reference

egui-winit suppresses Text events whenever Ctrl is held. For a focused native text field on Windows, recover text_with_all_modifiers only when Ctrl+Alt produced printable text different from the unmodified key. Send one Text event and skip shortcut dispatch for that press. Capture mode, web/media owners, key releases and ordinary shortcut/control characters retain their routes. The Legacy Viewer handles native translated Unicode text independently of shortcuts. A platform-independent character predicate test covers @, euro, control characters and non-layout shortcuts.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Windows required: use the affected ISO layout in chat, a search field and a script editor. Test Ctrl+Alt+2, right Alt/AltGr+2, Shift+2, euro and other AltGr symbols. Check no duplicate character, avatar motion or shortcut. Verify Ctrl+C/V, Ctrl+Alt+F9 and shortcut capture still work. Linux tests cover only the predicate; the Windows event-routing block must be compiled and exercised on Windows.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
