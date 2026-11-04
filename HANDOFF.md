# Issue #1854 — Include the link suffix in inventory name searches

Branch: `fix/inventory-link-search-1854`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/1854

## Change and reference

LLItemBridge::buildDisplayName appends the display suffix to the searchable name. Add (link) for links in Name and All text searches; retain the original cached name for display and sorting, and existing link inclusion/exclusion filters. Existing plus-separated query matching can now find name+link.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Create links to a known item. Search its name, then name+link; the latter should show links. Check Name and All search modes, links-only/exclude-links filters, a renamed link, a broken link, and a non-link item whose real name contains link. Ordinary name ordering should stay the same.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
