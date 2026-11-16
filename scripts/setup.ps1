<#
.SYNOPSIS
    Entry point of the one-line installation command: downloads the Aurora
    tools (scripts/tools) from GitHub, all at main's current commit, and
    runs the installation of the development environment. From a clone, runs the installation of that
    repository.

.DESCRIPTION
    The command, in a command prompt opened in the project folder:

      powershell -NoProfile -ExecutionPolicy Bypass -Command "$f = Join-Path $env:TEMP 'aurora-setup.ps1'; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f"

.EXAMPLE
    ./scripts/setup.ps1 -Path D:\Dev\Aurora-Viewer -Yes   # no questions
#>
param(
    # project folder (asked for when not given)
    [string]$Path,
    # no questions
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'

$local = Join-Path $PSScriptRoot 'tools\aurora-tools.ps1'
if (Test-Path $local) {
    & $local -Action install @PSBoundParameters
    exit
}

# Saved alone in %TEMP% by the one-liner: fetch every file of scripts/tools
# into a fresh folder, then run the installation. main's commit is read
# once, here, and every file comes from that commit: a push to main during
# the download cannot mix two versions of the tools.
$repo = 'Aurora-Viewer/Aurora-Viewer'
$commit = (Invoke-RestMethod "https://api.github.com/repos/$repo/commits/main").sha
if ($commit -notmatch '^[0-9a-f]{40}$') { throw "Could not read the commit of main ($commit)." }
$dir = Join-Path $env:TEMP 'aurora-tools'
if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
New-Item -ItemType Directory -Path $dir | Out-Null
Write-Host "Downloading the Aurora tools ($($commit.Substring(0, 9)))…"
$files = Invoke-RestMethod "https://api.github.com/repos/$repo/contents/scripts/tools?ref=$commit"
foreach ($f in $files | Where-Object { $_.type -eq 'file' }) {
    # A plain file name, written into $dir and nowhere else.
    if ($f.name -notmatch '^[A-Za-z0-9._-]+$' -or $f.name -match '^\.+$') { throw "Unexpected file name in scripts/tools: $($f.name)" }
    $url = "https://raw.githubusercontent.com/$repo/$commit/scripts/tools/$($f.name)"
    Invoke-WebRequest -Uri $url -OutFile (Join-Path $dir $f.name) -UseBasicParsing
}
& (Join-Path $dir 'aurora-tools.ps1') -Action install @PSBoundParameters
