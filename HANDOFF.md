# Issue #2255 — Allow emptying Trash with RLVa enabled

Branch: `fix/trash-rlva-2255`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2255

## Change and reference

LLInventoryModel::emptyFolderType (indra/newview/llinventorymodel.cpp) purges Trash without an RLVa lock veto. Exempt only the actual Trash folder from the mutation lock check. Other folder purges and deletions retain their lock checks; the Trash confirmation still refuses worn descendants.

## Automated checks

Formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

## Human finishing checks

With RLVa enabled, put disposable unworn items and a folder in Trash. Empty and confirm removal. Cancel first to verify no deletion. Check ordinary inventory actions still honor locked worn attachments and clothing. Use disposable inventory only.

No PR yet. Publish after human validation; upstream handles CI approval, review and merge. Upstream assignment/board writes are unavailable to the fork account.
