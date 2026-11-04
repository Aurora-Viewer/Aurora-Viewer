# Issue #2044 — camera search focus

Branch: `fix/camera-search-2044`
Base: `ae764307`
Issue: https://github.com/Aurora-Viewer/Aurora-Viewer/issues/2044

The filter search field called `keep_keys`, which immediately surrendered its keyboard focus after a click. Removed that call for the text field. Other controls retain their existing shortcut behavior. The Legacy Viewer’s LLLineEditor::handleMouseDown keeps keyboard focus when clicked; this is Aurora’s own filter UI.

Checks: formatting and diff review completed. Combined workspace validation is queued in `test/issue-fixes-validation`; no isolated branch build claimed. PowerShell is unavailable on this machine.

Human validation: open the camera/photo filters panel, click Search and type several letters and spaces. Confirm that the query appears, results filter, the caret stays active and the avatar/camera does not respond to those keys. Clear the query, then try the same search in the photo window’s Filters section. Check that clicking non-text look controls still works.

No PR yet. Assignment could not be updated with the fork account’s upstream permissions. Publish the PR after validation; upstream handles CI approval/review/merge.
