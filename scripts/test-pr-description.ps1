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
Assert-Misses $template 'the empty template' @('still the template''s `Closes #`', '`## What it is for`**: the heading is there', '`## Legacy Viewer origin`**: the heading is there', '`## What changes`**: it only holds', 'still unticked')

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
Assert-Misses ($good -replace 'Refs #2089', 'Closes #') 'the template issue line left as is' @('still the template''s `Closes #`')
Assert-Misses ($good -replace 'Refs #2089', 'Follow-up to #2620.') 'a follow-up sentence' @('put `Refs #2620` on the first line')
Assert-Misses ($good -replace 'Refs #2089', '<!-- Refs #1 -->') 'an issue line in a comment' @('`No issue` is perfectly fine')

# headings: "###", a longer wording, the past tense
Assert-Complete ($good -replace '(?m)^## ', '### ') 'level 3 headings'
Assert-Complete ($good -replace '## Tests', '## Tests and validation' -replace '## What changes', '## What changed') 'longer headings'
Assert-Misses ($good -replace '## Legacy Viewer origin', '## Origin') 'a renamed section' @('`## Legacy Viewer origin`**: this heading is missing (headings found: `## What it is for`, `## Origin`')

# sections left as the template has them
Assert-Misses ($good -replace 'In mouselook, a vehicle that steers toward the camera stops spinning\.', '<!-- 2 or 3 simple sentences. -->') 'an empty purpose' @('`## What it is for`**: the heading is there')
Assert-Misses ($good -replace '- \*\*Camera:\*\* the mouselook heading stays a world direction\.', '- **Interface:**') 'area placeholders only' @('it only holds the template''s empty bullets (`- **Interface:**`, `- **Rendering:**`)')
Assert-Misses ($good -replace '- \[x\] `scripts/check.ps1`', '- [ ] `scripts/check.ps1`') 'Tests boxes left unticked' @('still unticked')

# Tests written by the author, with or without boxes
Assert-Complete ($good -replace '- \[x\] `scripts/check.ps1` green \(fmt, clippy, tests\)', '- `scripts/check.ps1`: 7,956 passed, zero failures.') 'Tests as bullets'
Assert-Complete ($good -replace '- \[x\] `scripts/check.ps1` green \(fmt, clippy, tests\)', '- [ ] Validated by the human before the PR: 2026-11-08, tested on the grid.') 'an unticked box the author completed'
Assert-Complete ($good -replace '(?s)## Legacy Viewer origin\s+Yes:[^\n]+', "## Legacy Viewer origin`n`nNo.") 'a short "no" origin'

# no heading at all: one point, not four
Assert-Misses "Refs #1`n`nFixes the thing, tested in demo." 'a body with no heading' @('no `##` heading at all')
Assert-Pr ((@(Get-PrDescriptionProblems 'No issue')[0]).Contains('TEMPLATE_URL')) 'The no-heading point links the template.'

# the comment
$comment = Format-PrDescriptionComment @('A.', 'See [it](TEMPLATE_URL).') 'https://example.org/t' '2026-11-11' 'someone'
Assert-Pr ($comment.StartsWith('## Changes requested')) 'The comment must open with "## Changes requested" (Aurora Tools PR panel).'
Assert-Pr ($comment.TrimEnd().EndsWith($PrDescriptionMarker)) 'The comment must end with its marker.'
Assert-Pr ($comment -match '(?m)^1\. A\.$' -and $comment.Contains('2. See [it](https://example.org/t).')) 'The comment numbers each missing part and links the template.'
Assert-Pr ($comment.Contains('Hi @someone') -and $comment.Contains('2 things')) 'The comment greets the author and counts the points.'
Assert-Pr ($comment -match '2026-11-11') 'An outside contributor is given the closing date.'
Assert-Pr ((Format-PrDescriptionComment @('A.') 'u' $null) -notmatch 'closed') 'A team PR is never threatened with a close.'
Assert-Pr ((Format-PrDescriptionClose 'u' 'someone').Contains($PrDescriptionMarker) -and (Format-PrDescriptionClose 'u' 'someone').Contains('Hi @someone')) 'The closing comment greets the author and carries the marker.'

# who is never closed
Assert-Pr ((Test-PrTeamAuthor 'MEMBER') -and (Test-PrTeamAuthor 'OWNER') -and (Test-PrTeamAuthor 'COLLABORATOR')) 'The team is never closed.'
Assert-Pr (-not (Test-PrTeamAuthor 'CONTRIBUTOR') -and -not (Test-PrTeamAuthor 'FIRST_TIME_CONTRIBUTOR') -and -not (Test-PrTeamAuthor 'NONE')) 'Outside contributors may be closed.'

# GitHub refuses a label description over 100 characters: the workflow creates
# the label itself, so a longer one fails the first incomplete PR
$labelLines = @(Select-String -Path (Join-Path $PSScriptRoot 'setup-labels.ps1'), (Join-Path $PSScriptRoot 'check-pr-description.ps1') -Pattern "(?:Desc = |description=)'?([^']+)'" -AllMatches)
foreach ($m in $labelLines.Matches) {
    $d = $m.Groups[1].Value
    Assert-Pr ($d.Length -le 100) "Label description over 100 characters ($($d.Length)): $d"
}
Assert-Pr ($labelLines.Count -ge 2) 'The label descriptions were not found.'

Write-Host 'PR description fixtures: all passed.'
