<#
.SYNOPSIS
    Entry point of the one-line installation command: downloads the Aurora
    tools (scripts/tools) from GitHub and runs the installation of the
    development environment. From a clone, runs the installation of that
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
# from GitHub's main into a fresh folder, then run the installation.
$dir = Join-Path $env:TEMP 'aurora-tools'
if (Test-Path $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
New-Item -ItemType Directory -Path $dir | Out-Null
Write-Host 'Downloading the Aurora tools…'
$files = Invoke-RestMethod 'https://api.github.com/repos/Aurora-Viewer/Aurora-Viewer/contents/scripts/tools?ref=main'
foreach ($f in $files | Where-Object { $_.type -eq 'file' }) {
    Invoke-WebRequest -Uri $f.download_url -OutFile (Join-Path $dir $f.name) -UseBasicParsing
}
& (Join-Path $dir 'aurora-tools.ps1') -Action install @PSBoundParameters
