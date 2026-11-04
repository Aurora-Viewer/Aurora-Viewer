# Issue #2345 — Show the original inventory item from the Worn tab

Branch: `fix/worn-original-2345`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2345

## Change and reference

Match LLWearingContextMenu/menu_wearing_tab.xml: add the existing ShowOriginal action to the single-item Worn menu. Remove/Detach already exists and keeps its multi-selection and RLVa checks. Uses existing EN/FR/JA strings and the same original-item action as saved outfits.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Open Appearance > Worn. Right-click clothing, an attachment and a body part; Show original should select its real item in Inventory without requiring a saved outfit. For multiple selected items, Show original is absent and Remove/Detach continues acting on the removable selection. Verify body parts still cannot be removed and RLVa-locked items remain protected.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
