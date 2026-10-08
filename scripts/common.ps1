# Shared helpers for the scripts (dot-sourced: . "$PSScriptRoot\common.ps1").

# Native tools (cargo, git, gh) write progress to stderr: Windows PowerShell 5.1
# turns it into errors when the output is redirected. They are judged on their
# exit code instead (Invoke-Checked / $LASTEXITCODE); cmdlets that must not
# fail silently use -ErrorAction Stop.
$ErrorActionPreference = 'Continue'

# Main repository (also when called from a worktree) and its parent folder,
# which holds work\ and phoenix-firestorm\.
function Get-AuroraPaths {
    $common = (git rev-parse --path-format=absolute --git-common-dir).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $common) { throw "Pas dans un dépôt git Aurora Viewer." }
    $main = Split-Path $common -Parent
    $root = Split-Path $main -Parent
    [pscustomobject]@{ Main = $main; Root = $root; Work = (Join-Path $root 'work') }
}

# Task name -> folder / branch name: lowercase letters, digits and dashes.
function Get-TaskSlug([string]$Name) {
    $slug = ($Name.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')
    if (-not $slug) { throw "Nom de tâche invalide : '$Name'." }
    $slug
}

# Run a native command and stop on failure.
function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Échec : $What (code $LASTEXITCODE)." }
}
