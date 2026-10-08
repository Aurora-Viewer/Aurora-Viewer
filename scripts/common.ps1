# Shared helpers for the scripts (dot-sourced: . "$PSScriptRoot\common.ps1").

# Native tools (cargo, git, gh) write progress to stderr: Windows PowerShell 5.1
# turns it into errors when the output is redirected. They are judged on their
# exit code instead (Invoke-Checked / $LASTEXITCODE); cmdlets that must not
# fail silently use -ErrorAction Stop.
$ErrorActionPreference = 'Continue'

# Main repository (also when called from a worktree) and its parent folder,
# which holds work\ and phoenix-firestorm\. Found from the scripts' own
# folder, so a script works whatever the current folder (double-click…).
function Get-AuroraPaths {
    $common = (git -C $PSScriptRoot rev-parse --path-format=absolute --git-common-dir).Trim()
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

# A build or a viewer is running from $Dir: compiler command lines name
# their output folder, the viewer runs from its own exe.
function Test-InUse([string]$Dir) {
    $full = [IO.Path]::GetFullPath($Dir).TrimEnd('\') + '\'
    $procs = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue -Filter (
        "Name='cargo.exe' OR Name='rustc.exe' OR Name='clippy-driver.exe' OR Name='aurora-viewer.exe'")
    [bool]($procs | Where-Object {
        ($_.ExecutablePath -and $_.ExecutablePath.StartsWith($full, [StringComparison]::OrdinalIgnoreCase)) -or
        ($_.CommandLine -and $_.CommandLine.IndexOf($full, [StringComparison]::OrdinalIgnoreCase) -ge 0)
    })
}

# Run a native command and stop on failure.
function Invoke-Checked([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Échec : $What (code $LASTEXITCODE)." }
}
