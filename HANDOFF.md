# Issue #2303 — Use the drawn viewpoint for Create shape silhouettes

Branch: `fix/build-shape-icons-2303`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2303

## Change and reference

The old icon silhouette used a separate perspective eye while the painter projected an orthographic X/Z view after rotation. Compute facing from that actual projection and include its rotation in the preview cache key. LLVolume::generateSilhouetteVertices likewise derives facing from the actual object-space view. The reported image was inspected.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

Open Build > Create in both interface layouts. Inspect cone, sphere, half-sphere, torus, tube and ring, including their rotated variants. Icons should have coherent outlines rather than zigzags or internal cuts. Reopen the palette and vary UI scale to check cached shapes and clipping. This changes icon previews only, not world geometry.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
