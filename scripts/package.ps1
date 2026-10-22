<#
.SYNOPSIS
    Puts the built viewer and its files in a folder ready to launch (or to
    zip): exe, assets, licences. The GitHub release and build-release.ps1
    both use it: their folders are identical. The Linux job of the release
    runs it too (PowerShell 7), with the executable without extension.

.EXAMPLE
    ./scripts/package.ps1 -Exe target/release/aurora-viewer.exe -Out Aurora-Viewer
.EXAMPLE
    ./scripts/package.ps1 -Exe target/release/aurora-viewer -Out Aurora-Viewer-v0.8.0-linux-x64
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    # repository whose assets go with the exe (default: this script's)
    [string]$Source = (Split-Path $PSScriptRoot -Parent)
)

New-Item -ItemType Directory -Force -Path (Join-Path $Out 'assets/emoji') -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath $Exe -Destination $Out -ErrorAction Stop
# The emoji font is inside the exe; its licence ships, and the assets folder
# stays: the updater of the published versions refuses a package without it.
Copy-Item -LiteralPath (Join-Path $Source 'assets/emoji/LICENSE-OFL.txt') -Destination (Join-Path $Out 'assets/emoji') -ErrorAction Stop
$docs = 'LICENSE', 'NOTICE.md', 'README.md' | ForEach-Object { Join-Path $Source $_ }
Copy-Item -LiteralPath $docs -Destination $Out -ErrorAction Stop
$taskExePath = (Resolve-Path -LiteralPath $Exe).Path
$taskVersion = (& $taskExePath --version | Out-String).Trim() -replace '^Aurora Viewer ', ''
if ($LASTEXITCODE -ne 0 -or $taskVersion -notmatch '^\d+\.\d+\.\d+([+-][0-9A-Za-z.+-]+)?$') { throw 'Invalid package version.' }
$taskJson = @{ schema = 1; version = $taskVersion } | ConvertTo-Json
$taskMetadataPath = Join-Path (Resolve-Path -LiteralPath $Out -ErrorAction Stop).Path 'aurora-install.json'
# Windows PowerShell 5.1 does not support Set-Content -Encoding utf8NoBOM.
[IO.File]::WriteAllText($taskMetadataPath, $taskJson, [Text.UTF8Encoding]::new($false))
