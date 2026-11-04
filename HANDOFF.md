# Issue #2016 — Preserve a profile pick parcel while editing its contents

Branch: `fix/profile-pick-location-2016`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2016

## Change and reference

LLPanelProfilePick::sendUpdate retains mParcelId. Keep the received parcel UUID in PickEdit and copy it into PickInfo on save; previously it silently defaulted to null. Set Location explicitly replaces both position and parcel state, while new picks retain the existing null-parcel creation behavior. A regression checks that editing name, description and snapshot preserves the saved parcel and coordinates.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Grid required: stand somewhere different from an existing pick. Change only its name/description, save and reopen; its original location must remain. Change its photo and repeat. Then click Set Location deliberately, save and reopen; it should move to the current place. Check new-pick creation and both profile layouts. The agent has not connected to a grid.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
