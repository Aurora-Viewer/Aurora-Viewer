# Review slot fixtures (scripts/tools/pr-slots.ps1): one author's open PRs
# and what the plan does with them. No GitHub call; the Review slots
# workflow runs it on the pull requests that change the slots.
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/tools/pr-slots.ps1"
function Assert-Slot([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Assert-Set($Actual, [int[]]$Expected, [string]$Name) {
    $a = (@($Actual) | Sort-Object) -join ','
    $e = (@($Expected) | Sort-Object) -join ','
    Assert-Slot ($a -eq $e) "${Name}: expected [$e], got [$a]"
}
# PR $n created on day $n (older = smaller number)
function New-Pr([int]$n, [switch]$Draft, [switch]$Waiting, [switch]$OnHold, [switch]$Protected, [switch]$Reviewed) {
    [pscustomobject]@{ Number = $n; Draft = [bool]$Draft; Waiting = [bool]$Waiting; OnHold = [bool]$OnHold
        Protected = [bool]$Protected; Reviewed = [bool]$Reviewed; Created = ('2026-11-{0:d2}T00:00:00Z' -f $n) }
}

# under the limit: nothing moves
$p = Get-PrSlotPlan @(1..3 | ForEach-Object { New-Pr $_ }) 3
Assert-Set $p.Wait @() 'at the limit'
Assert-Set $p.Back @() 'at the limit, back'

# over the limit: the newest wait
$p = Get-PrSlotPlan @(1..5 | ForEach-Object { New-Pr $_ }) 3
Assert-Set $p.Wait @(4, 5) 'the newest wait'

# a PR a maintainer started reviewing keeps its slot, even if it is newer
$p = Get-PrSlotPlan @((1..4 | ForEach-Object { New-Pr $_ }) + (New-Pr 5 -Reviewed)) 3
Assert-Set $p.Wait @(3, 4) 'a reviewed PR keeps its slot'

# a PR in the merge queue never waits, even beyond the limit
$p = Get-PrSlotPlan @((1..3 | ForEach-Object { New-Pr $_ -Reviewed }) + (New-Pr 4 -Protected)) 3
Assert-Set $p.Wait @(3) 'a protected PR keeps its slot'
$p = Get-PrSlotPlan @(1..4 | ForEach-Object { New-Pr $_ -Protected }) 3
Assert-Set $p.Wait @() 'protected PRs never wait'

# the one whose label a maintainer just removed takes a slot; the newest other waits
$items = @((1..3 | ForEach-Object { New-Pr $_ }) + (New-Pr 4))
$p = Get-PrSlotPlan $items 3 4
Assert-Set $p.Wait @(3) 'the preferred PR stays in review'

# a free slot: the oldest waiting comes back
$items = @((New-Pr 1), (New-Pr 2), (New-Pr 5 -Waiting), (New-Pr 4 -Waiting))
$p = Get-PrSlotPlan $items 3
Assert-Set $p.Back @(4) 'the oldest waiting comes back'
Assert-Set $p.Wait @() 'nothing waits when a slot is free'

# stable: a newer PR in review is not swapped for an older waiting one
$items = @((New-Pr 2), (New-Pr 3), (New-Pr 9), (New-Pr 1 -Waiting))
$p = Get-PrSlotPlan $items 3
Assert-Set $p.Wait @() 'no swap at the limit'
Assert-Set $p.Back @() 'no swap at the limit, back'

# drafts and PRs on hold take no slot and are left alone
$items = @((New-Pr 1), (New-Pr 2), (New-Pr 3 -Draft), (New-Pr 4 -OnHold), (New-Pr 5 -Waiting))
$p = Get-PrSlotPlan $items 3
Assert-Set $p.Back @(5) 'drafts and on-hold take no slot'
Assert-Set $p.Wait @() 'drafts and on-hold never wait'
Assert-Set $p.Unlabel @() 'nothing to unlabel'

# a waiting PR its author turned into a draft, or put on hold: only the label goes
$items = @((New-Pr 1), (New-Pr 2 -Waiting -Draft), (New-Pr 3 -Waiting -OnHold))
$p = Get-PrSlotPlan $items 3
Assert-Set $p.Unlabel @(2, 3) 'a waiting draft or on-hold PR loses the label'
Assert-Set $p.Back @() 'a draft does not come back'

# the comments
$links = Get-PrSlotLinks 'Aurora-Viewer/Aurora-Viewer' 'someone'
Assert-Slot ($links.Review.Contains('author%3Asomeone') -and $links.Review.Contains('-is%3Adraft') -and $links.Review.Contains("-label%3A$PrSlotLabel")) 'The review link lists the author''s PRs in review.'
Assert-Slot ($links.Waiting.Contains("label%3A$PrSlotLabel")) 'The waiting link lists the waiting PRs.'
$wait = Format-PrSlotWait 'someone' 42 $links
Assert-Slot ($wait.StartsWith('## Waiting for a review slot')) 'The waiting comment opens with its heading.'
Assert-Slot ($wait.Contains("``$PrSlotLabel``") -and -not $wait.Contains('draft**')) 'The waiting comment names the label, not a draft.'
Assert-Slot ($wait.Contains('Hi @someone') -and $wait.Contains('**42** open') -and $wait.Contains("**$PrSlotLimit pull requests in review**")) 'The waiting comment names the author, the count and the limit.'
Assert-Slot ($wait.TrimEnd().EndsWith($PrSlotMarker)) 'The waiting comment ends with its marker.'
$back = Format-PrSlotBack 'someone'
Assert-Slot ($back.Contains('Hi @someone') -and $back.Contains($PrSlotMarker)) 'The back comment names the author and carries the marker.'
Assert-Slot ((Test-PrSlotTeam 'MEMBER') -and -not (Test-PrSlotTeam 'CONTRIBUTOR')) 'Only the team is exempt.'

# GitHub refuses a label description over 100 characters
$desc = [regex]::Match([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'pr-slots.ps1')), "description=([^']+)'").Groups[1].Value
Assert-Slot ($desc -and $desc.Length -le 100) "The label description is missing or over 100 characters: $desc"

Write-Host 'Review slot fixtures: all passed.'
