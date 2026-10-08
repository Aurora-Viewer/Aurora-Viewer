<#
.SYNOPSIS
    Supprime le dossier de travail d'une tâche terminée (worktree, branche
    locale, target\). Refuse si la PR n'est pas fusionnée ou s'il reste des
    changements non commités, sauf avec -Force.

.DESCRIPTION
    Si un programme utilise encore le dossier (terminal, éditeur, viewer lancé
    depuis la tâche), git retire le worktree mais ne peut pas effacer le
    dossier : le script le dit clairement, et une nouvelle exécution termine
    le ménage une fois le dossier libéré.

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
if (-not (Test-Path -LiteralPath $dir)) { throw "Pas de dossier de travail $dir." }
Set-Location $p.Main

$isWorktree = Test-Path -LiteralPath (Join-Path $dir '.git')
if ($isWorktree) {
    $branch = (git -C $dir rev-parse --abbrev-ref HEAD).Trim()
} else {
    # leftover of an interrupted clean-up: an empty folder or a copy of this repository
    $empty = -not (Get-ChildItem -LiteralPath $dir -Force | Select-Object -First 1)
    $copy = (Test-Path -LiteralPath (Join-Path $dir 'Cargo.toml')) -and (Test-Path -LiteralPath (Join-Path $dir 'AGENTS.md'))
    if (-not ($empty -or $copy)) {
        throw "$dir n'est ni un worktree ni un reste de tâche Aurora : rien n'est supprimé."
    }
    $branch = git branch --list "*/$slug" --format '%(refname:short)' | Select-Object -First 1
}

if (-not $Force -and $isWorktree) {
    if (git -C $dir status --porcelain) { throw "Changements non commités dans $dir (utilise -Force pour les abandonner)." }
    # squash merges are not ancestors of main: ask GitHub whether the PR is merged
    $state = gh pr view $branch --json state --jq .state 2>$null
    if ($state -ne 'MERGED') { throw "La PR de $branch n'est pas fusionnée (état : '$state'). Utilise -Force pour supprimer quand même." }
}

if ($isWorktree) {
    $gitArgs = @('worktree', 'remove', $dir)
    if ($Force) { $gitArgs += '--force' }
    git @gitArgs
}
git worktree prune
if ((git worktree list --porcelain) -contains "worktree $($dir -replace '\\', '/')") {
    throw "git n'a pas pu retirer le worktree $dir (voir le message ci-dessus)."
}
# git may leave the folder behind when it is in use: delete what remains
if (Test-Path -LiteralPath $dir) {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
if ($branch) { git branch -D $branch 2>$null | Out-Null }

if (Test-Path -LiteralPath $dir) {
    Write-Warning ("Le dossier $dir n'a pas pu être entièrement supprimé : un programme l'utilise " +
        "(terminal ou éditeur ouvert dedans, viewer lancé depuis la tâche). Ferme-le, puis relance " +
        "./scripts/end-task.ps1 -Name $slug pour finir.")
    exit 1
}
Write-Host "Tâche $slug nettoyée$(if ($branch) { " (branche $branch supprimée en local)" })." -ForegroundColor Green
