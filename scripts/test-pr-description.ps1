# PR description check fixtures (scripts/tools/pr-description.ps1): bodies
# as authors write them, and the template itself. No GitHub call; the PR
# description workflow runs it on the pull requests that change the check.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/pr-description.ps1"
function Assert-Pr([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Assert-Complete([string]$Body, [string]$Name) {
    $p = @(Get-PrDescriptionProblems $Body)
    Assert-Pr ($p.Count -eq 0) "$Name should be complete, got: $($p -join ' / ')"
}
function Assert-Misses([string]$Body, [string]$Name, [string[]]$Patterns) {
    $p = @(Get-PrDescriptionProblems $Body)
    Assert-Pr ($p.Count -eq $Patterns.Count) "$Name should miss $($Patterns.Count) parts, got $($p.Count): $($p -join ' / ')"
    foreach ($pattern in $Patterns) {
        # Contains, not -like: the backticks of the messages are -like's escape
        Assert-Pr ([bool]($p | Where-Object { $_.Contains($pattern) })) "$Name should miss '$pattern', got: $($p -join ' / ')"
    }
}

$good = @'
Refs #2089

## What it is for

In mouselook, a vehicle that steers toward the camera stops spinning.

## Legacy Viewer origin

Yes: `LLAgentCamera::calcFocusPositionTargetGlobal` (indra/newview/llagentcamera.cpp).

## What changes

- **Camera:** the mouselook heading stays a world direction.
- **Rendering:**

## Screenshots

## Tests

- [x] `scripts/check.ps1` green (fmt, clippy, tests)
- [ ] Demo scenarios:

## Limits and follow-up
'@
Assert-Complete $good 'a filled-in template'

# the template as GitHub opens it: every part missing
$template = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../.github/pull_request_template.md'))
Assert-Misses $template 'the empty template' @('issue line', '## What it is for', '## Legacy Viewer origin', '## What changes', '## Tests')

# every required heading is in the template, so the two cannot drift apart
$sections = Get-PrSections $template
foreach ($name in $PrRequiredSections) {
    Assert-Pr ($null -ne (Find-PrSection $sections $name)) "The template has no '## $name' heading."
}

# the issue line
Assert-Complete ($good -replace 'Refs #2089', 'No issue (follows #2063)') 'No issue with a note'
Assert-Complete ($good -replace 'Refs #2089', 'Closes #12') 'Closes'
Assert-Complete ($good -replace 'Refs #2089', 'Fixes Aurora-Viewer/Aurora-Viewer#12') 'a cross-repository reference'
Assert-Complete ($good -replace 'Refs #2089', 'Refs https://github.com/Aurora-Viewer/Aurora-Viewer/issues/12') 'an issue link'
Assert-Misses ($good -replace 'Refs #2089', 'Closes #') 'the template issue line left as is' @('issue line')
Assert-Misses ($good -replace 'Refs #2089', 'Follow-up to #2620.') 'a follow-up sentence' @('issue line')
Assert-Misses ($good -replace 'Refs #2089', '<!-- Refs #1 -->') 'an issue line in a comment' @('issue line')

# headings: "###", a longer wording, the past tense
Assert-Complete ($good -replace '(?m)^## ', '### ') 'level 3 headings'
Assert-Complete ($good -replace '## Tests', '## Tests and validation' -replace '## What changes', '## What changed') 'longer headings'
Assert-Misses ($good -replace '## Legacy Viewer origin', '## Origin') 'a renamed section' @('## Legacy Viewer origin')

# sections left as the template has them
Assert-Misses ($good -replace 'In mouselook, a vehicle that steers toward the camera stops spinning\.', '<!-- 2 or 3 simple sentences. -->') 'an empty purpose' @('under `## What it is for`')
Assert-Misses ($good -replace '- \*\*Camera:\*\* the mouselook heading stays a world direction\.', '- **Interface:**') 'area placeholders only' @('under `## What changes`')
Assert-Misses ($good -replace '- \[x\] `scripts/check.ps1`', '- [ ] `scripts/check.ps1`') 'Tests boxes left unticked' @('Under `## Tests`')

# Tests written by the author, with or without boxes
Assert-Complete ($good -replace '- \[x\] `scripts/check.ps1` green \(fmt, clippy, tests\)', '- `scripts/check.ps1`: 7,956 passed, zero failures.') 'Tests as bullets'
Assert-Complete ($good -replace '- \[x\] `scripts/check.ps1` green \(fmt, clippy, tests\)', '- [ ] Validated by the human before the PR: 2026-11-08, tested on the grid.') 'an unticked box the author completed'
Assert-Complete ($good -replace '(?s)## Legacy Viewer origin\s+Yes:[^\n]+', "## Legacy Viewer origin`n`nNo.") 'a short "no" origin'

# the comment
$comment = Format-PrDescriptionComment @('A.', 'B.') 'https://example.org/t' '2026-11-11'
Assert-Pr ($comment.StartsWith('## Changes requested')) 'The comment must open with "## Changes requested" (Aurora Tools PR panel).'
Assert-Pr ($comment.TrimEnd().EndsWith($PrDescriptionMarker)) 'The comment must end with its marker.'
Assert-Pr ($comment -match '(?m)^- A\.$' -and $comment -match '(?m)^- B\.$') 'The comment lists each missing part.'
Assert-Pr ($comment -match '2026-11-11') 'An outside contributor is given the closing date.'
Assert-Pr ((Format-PrDescriptionComment @('A.') 'u' $null) -notmatch 'closed') 'A team PR is never threatened with a close.'
Assert-Pr ((Format-PrDescriptionClose 'u').Contains($PrDescriptionMarker)) 'The closing comment carries the marker.'

# who is never closed
Assert-Pr ((Test-PrTeamAuthor 'MEMBER') -and (Test-PrTeamAuthor 'OWNER') -and (Test-PrTeamAuthor 'COLLABORATOR')) 'The team is never closed.'
Assert-Pr (-not (Test-PrTeamAuthor 'CONTRIBUTOR') -and -not (Test-PrTeamAuthor 'FIRST_TIME_CONTRIBUTOR') -and -not (Test-PrTeamAuthor 'NONE')) 'Outside contributors may be closed.'

Write-Host 'PR description fixtures: all passed.'
