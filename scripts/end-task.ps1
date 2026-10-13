<#
.SYNOPSIS
    Deletes the work folder of a finished task (worktree, local branch,
    target\). Refuses if the PR is not merged or if uncommitted changes
    remain, except with -Force.

.DESCRIPTION
    If a program is still using the folder (terminal, editor, viewer launched
    from the task), git removes the worktree but cannot delete the folder:
    the script says so clearly, and running it again finishes the cleanup
    once the folder is free.

.EXAMPLE
    ./scripts/end-task.ps1 -Name avatar-gaze
#>
param(
    [Parameter(Mandatory)][string]$Name,
    [switch]$Force
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$slug = Get-TaskSlug $Name
$dir = Join-Path $p.Work $slug
if (-not (Test-Path -LiteralPath $dir)) { throw "No work folder $dir." }
# Leave the task folder, PowerShell location and process working folder
# alike: Set-Location alone keeps the process inside it, and Windows then
# refuses to delete the folder (run from the task, as agents often are).
Set-Location $p.Main
[Environment]::CurrentDirectory = $p.Main

$isWorktree = Test-Path -LiteralPath (Join-Path $dir '.git')
if ($isWorktree) {
    $branch = (git -C $dir rev-parse --abbrev-ref HEAD).Trim()
} else {
    # leftover of an interrupted clean-up: an empty folder or a copy of this repository
    $empty = -not (Get-ChildItem -LiteralPath $dir -Force | Select-Object -First 1)
    $copy = (Test-Path -LiteralPath (Join-Path $dir 'Cargo.toml')) -and (Test-Path -LiteralPath (Join-Path $dir 'AGENTS.md'))
    if (-not ($empty -or $copy)) {
        throw "$dir is neither a worktree nor a leftover of an Aurora task: nothing is deleted."
    }
    $branch = git branch --list "*/$slug" --format '%(refname:short)' | Select-Object -First 1
}

if (-not $Force -and $isWorktree) {
    if (git -C $dir status --porcelain) { throw "Uncommitted changes in $dir (use -Force to discard them)." }
    # squash merges are not ancestors of main: ask GitHub whether the PR is merged
    $state = gh pr view $branch --json state --jq .state 2>$null
    if ($state -ne 'MERGED') { throw "The PR of $branch is not merged (state: '$state'). Use -Force to delete anyway." }
}

if ($isWorktree) {
    $gitArgs = @('worktree', 'remove', $dir)
    if ($Force) { $gitArgs += '--force' }
    git @gitArgs
}
git worktree prune
if ((git worktree list --porcelain) -contains "worktree $($dir -replace '\\', '/')") {
    throw "git could not remove the worktree $dir (see the message above)."
}
# git may leave the folder behind when it is in use: delete what remains
if (Test-Path -LiteralPath $dir) {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
if ($branch) { git branch -D $branch 2>$null | Out-Null }

if (Test-Path -LiteralPath $dir) {
    Write-Warning ("The folder $dir could not be fully deleted: a program is using it " +
        "(terminal or editor open in it, viewer launched from the task). Close it, then run " +
        "./scripts/end-task.ps1 -Name $slug again to finish.")
    exit 1
}
Write-Host "Task $slug cleaned up$(if ($branch) { " (local branch $branch deleted)" })." -ForegroundColor Green
