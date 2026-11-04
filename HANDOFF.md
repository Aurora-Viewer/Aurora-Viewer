# Issue #1943 — Mask texture picker UUIDs without full inventory permissions

Branch: `fix/texture-uuid-permissions-1943`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/1943

## Change and reference

Match LLFloaterTexturePicker::onSelectionChange: only reveal an inventory asset UUID when the current agent has Copy, Modify and Transfer. Resolve link permissions from the original. Unknown/restricted assets show the null UUID without replacing the selected texture. Local assets and public preset/BOM constants remain visible; manually entered UUID text remains editable. Reset manual-input state on selection/reopen, and refresh permission masking immediately before drawing the field. Both report images were inspected. Tests cover each missing right, links, foreign-owner masks, unknown assets, selection preservation and explicit input.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Open a non-full-permission wearable texture and an object-face texture absent from your full-permission inventory: the UUID field should show zeros and copying it must not reveal the key. Test no-copy, no-modify and no-transfer inventory textures and links, then a full-permission texture. Check Blank/Transparent/BOM/local choices, a manually pasted UUID, Cancel/OK, and reopening on a restricted asset after pasting. Verify masked IDs never silently select the null texture.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
