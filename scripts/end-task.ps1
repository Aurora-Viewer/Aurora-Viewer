<#
.SYNOPSIS
    Supprime le dossier de travail d'une tâche terminée (worktree, branche
    locale, target\). Refuse si la PR n'est pas fusionnée ou s'il reste des
    changements non commités, sauf avec -Force.

.EXAMPLE
    ./scripts/end-task.ps1 -Name regard-des-avatars
#>
param(
    [Parameter(Mandatory)][string]$Name,
    [switch]$Force
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$slug = Get-TaskSlug $Name
$dir = Join-Path $p.Work $slug
if (-not (Test-Path $dir)) { throw "Pas de dossier de travail $dir." }

$branch = (git -C $dir rev-parse --abbrev-ref HEAD).Trim()
if (-not $Force) {
    if (git -C $dir status --porcelain) { throw "Changements non commités dans $dir (utilise -Force pour les abandonner)." }
    # squash merges are not ancestors of main: ask GitHub whether the PR is merged
    $state = gh pr view $branch --json state --jq .state 2>$null
    if ($state -ne 'MERGED') { throw "La PR de $branch n'est pas fusionnée (état : '$state'). Utilise -Force pour supprimer quand même." }
}

Set-Location $p.Main
$gitArgs = @('worktree', 'remove', $dir)
if ($Force) { $gitArgs += '--force' }
Invoke-Checked 'git worktree remove' { git @gitArgs }
git branch -D $branch | Out-Null
git worktree prune
Write-Host "Tâche $slug nettoyée (branche $branch supprimée en local)." -ForegroundColor Green
