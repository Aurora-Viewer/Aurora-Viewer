<#
.SYNOPSIS
    Puts a ready PR in the merge queue and waits for the outcome: merged, or
    out of the queue (red CI on the code combined with the latest main,
    conflict…). Run it only when AGENTS.md allows it (section 8, step 9).

.EXAMPLE
    ./scripts/merge-pr.ps1 -Pr 42
#>
param(
    [Parameter(Mandatory)][int]$Pr,
    [int]$TimeoutMinutes = 90,
    [int]$GraceSeconds = 60
)
. "$PSScriptRoot\common.ps1"

# The GitHub repository, from this repository's origin: every gh call names
# it, so the script never moves the caller's terminal (a terminal left in a
# task folder keeps it from being deleted).
$origin = (git -C $PSScriptRoot remote get-url origin).Trim()
if ($origin -notmatch 'github\.com[:/](?<owner>[^/]+)/(?<name>[^/]+?)(\.git)?$') { throw "Unexpected origin remote: $origin" }
$owner = $Matches.owner; $name = $Matches.name
# queue entry and auto-merge request: once both are gone while the PR is
# still open, the queue has let it go
$query = 'query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){pullRequest(number:$n){state mergedAt mergeQueueEntry{state} autoMergeRequest{enabledAt}}}}'
function Get-PrState {
    $j = gh api graphql -f query=$query -F o=$owner -F r=$name -F n=$Pr | ConvertFrom-Json
    $j.data.repository.pullRequest
}

function Exit-Merged($s) {
    $at = if ($s.mergedAt) { " ($(([datetime]$s.mergedAt).ToLocalTime().ToString('HH:mm:ss')))" } else { '' }
    Write-Host "PR #$Pr merged$at." -ForegroundColor Green
    exit 0
}

# A successful queue run removes the entry a few seconds before GitHub marks
# the PR MERGED (PR #46: entry gone, state still OPEN, MERGED seconds later):
# keep asking for a while before calling it a queue exit. $null while the PR
# is still open and back in the queue (or on auto-merge).
function Wait-LeftQueue {
    $until = (Get-Date).AddSeconds($GraceSeconds)
    $last = $null
    while ((Get-Date) -lt $until) {
        Start-Sleep -Seconds 5
        $s = Get-PrState
        if (-not $s) { continue }
        if ($s.state -ne 'OPEN') { return $s }
        if ($s.mergeQueueEntry -or $s.autoMergeRequest) { return $null }
        $last = $s
    }
    $last
}

# never --admin: the protection of main applies to everyone
Invoke-Checked 'gh pr merge' { gh pr merge $Pr --squash --auto -R "$owner/$name" }
Write-Host "PR #$Pr waiting to merge (CI of the PR, then merge queue)…"

$deadline = (Get-Date).AddMinutes($TimeoutMinutes)
$waiting = $false
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 30
    $s = Get-PrState
    if (-not $s) { continue }
    if ($s.mergeQueueEntry -and -not $waiting) {
        Write-Host "In the queue: tests on the code combined with the latest main…"
        $waiting = $true
    }
    if ($s.state -eq 'OPEN' -and -not $s.mergeQueueEntry -and -not $s.autoMergeRequest) {
        Write-Host "PR #$Pr no longer in the queue: checking the merge (up to $GraceSeconds s)…"
        $left = Wait-LeftQueue
        if (-not $left) { continue }
        $s = $left
    }
    if ($s.state -eq 'MERGED') { Exit-Merged $s }
    if ($s.state -eq 'CLOSED') { throw "PR #$Pr closed without being merged." }
    if (-not $s.mergeQueueEntry -and -not $s.autoMergeRequest) {
        Write-Host "PR #$Pr left the queue without being merged. Latest CI runs of the queue:" -ForegroundColor Red
        gh run list --workflow CI --event merge_group --limit 3 -R "$owner/$name"
        exit 1
    }
}
throw "PR #$Pr not merged after $TimeoutMinutes min: check the queue on GitHub."
