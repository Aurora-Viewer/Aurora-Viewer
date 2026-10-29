<#
.SYNOPSIS
    Builds "Aurora Viewer.app" from the built viewer: the executable in
    Contents/MacOS, the files of package.ps1 (licences, notices, install
    metadata) with the icon and the translated texts of Info.plist in
    Contents/Resources. The macOS job of the release runs it (PowerShell 7);
    signing and notarization come after, in the workflow.
.EXAMPLE
    ./scripts/package-macos.ps1 -Exe target/release/aurora-viewer -Out "Aurora Viewer.app"
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    # repository whose files go in the bundle (default: this script's)
    [string]$Source = (Split-Path $PSScriptRoot -Parent)
)
$ErrorActionPreference = 'Stop'

if (Test-Path -LiteralPath $Out) { Remove-Item -LiteralPath $Out -Recurse -Force }
$contents = Join-Path $Out 'Contents'
$macos = Join-Path $contents 'MacOS'
$resources = Join-Path $contents 'Resources'
New-Item -ItemType Directory -Force -Path $macos, $resources | Out-Null

# the files every package carries, then the executable where macOS runs it
# (code only in Contents/MacOS: the signature seals the rest as resources)
& (Join-Path $PSScriptRoot 'package.ps1') -Exe $Exe -Out $resources -Source $Source
Move-Item -LiteralPath (Join-Path $resources (Split-Path $Exe -Leaf)) -Destination (Join-Path $macos 'aurora-viewer')

# MAJOR.MINOR.PATCH of the build, the only form CFBundleShortVersionString
# takes (a beta's pre-release part and the commits since the tag are left out)
$version = (Get-Content -LiteralPath (Join-Path $resources 'aurora-install.json') -Raw | ConvertFrom-Json).version
if ($version -notmatch '^(\d+\.\d+\.\d+)') { throw "Invalid package version: $version" }
$plist = Get-Content -LiteralPath (Join-Path $Source 'packaging/macos/Info.plist') -Raw
Set-Content -LiteralPath (Join-Path $contents 'Info.plist') -Value $plist.Replace('@VERSION@', $Matches[1]) -Encoding utf8NoBOM -NoNewline
Set-Content -LiteralPath (Join-Path $contents 'PkgInfo') -Value 'APPL????' -Encoding ascii -NoNewline

Copy-Item -LiteralPath (Join-Path $Source 'assets/branding/aurora.icns') -Destination (Join-Path $resources 'Aurora.icns')
Get-ChildItem -LiteralPath (Join-Path $Source 'packaging/macos') -Directory -Filter '*.lproj' | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $resources -Recurse
}
"$Out: Aurora Viewer $version"
