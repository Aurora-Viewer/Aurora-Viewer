<#
.SYNOPSIS
    Met une PR prête dans la file de fusion (merge queue) et attend l'issue :
    fusionnée, ou sortie de la file (CI rouge sur le code combiné avec le
    dernier main, conflit…). À lancer seulement quand AGENTS.md le permet
    (section 8, étape 9).

.EXAMPLE
    ./scripts/merge-pr.ps1 -Pr 42
#>
param(
    [Parameter(Mandatory)][int]$Pr,
    [int]$TimeoutMinutes = 90
)
. "$PSScriptRoot\common.ps1"

# The GitHub repository, from this repository's origin: every gh call names
# it, so the script never moves the caller's terminal (a terminal left in a
# task folder keeps it from being deleted).
$origin = (git -C $PSScriptRoot remote get-url origin).Trim()
if ($origin -notmatch 'github\.com[:/](?<owner>[^/]+)/(?<name>[^/]+?)(\.git)?$') { throw "Remote origin inattendu : $origin" }
$owner = $Matches.owner; $name = $Matches.name
# queue entry and auto-merge request: once both are gone while the PR is
# still open, the queue has let it go
$query = 'query($o:String!,$r:String!,$n:Int!){repository(owner:$o,name:$r){pullRequest(number:$n){state mergeQueueEntry{state} autoMergeRequest{enabledAt}}}}'
function Get-PrState {
    $j = gh api graphql -f query=$query -F o=$owner -F r=$name -F n=$Pr | ConvertFrom-Json
    $j.data.repository.pullRequest
}

# never --admin: the protection of main applies to everyone
Invoke-Checked 'gh pr merge' { gh pr merge $Pr --squash --auto -R "$owner/$name" }
Write-Host "PR #$Pr en attente de fusion (CI de la PR, puis file de fusion)…"

$deadline = (Get-Date).AddMinutes($TimeoutMinutes)
$waiting = $false
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 30
    $s = Get-PrState
    if (-not $s) { continue }
    if ($s.state -eq 'MERGED') {
        Write-Host "PR #$Pr fusionnée." -ForegroundColor Green
        exit 0
    }
    if ($s.state -eq 'CLOSED') { throw "PR #$Pr fermée sans être fusionnée." }
    if ($s.mergeQueueEntry -and -not $waiting) {
        Write-Host "Dans la file : tests sur le code combiné avec le dernier main…"
        $waiting = $true
    }
    if (-not $s.mergeQueueEntry -and -not $s.autoMergeRequest) {
        Write-Host "PR #$Pr sortie de la file sans être fusionnée. Dernière CI de la file :" -ForegroundColor Red
        gh run list --workflow CI --event merge_group --limit 3 -R "$owner/$name"
        exit 1
    }
}
throw "PR #$Pr pas fusionnée après $TimeoutMinutes min : vérifie la file sur GitHub."
