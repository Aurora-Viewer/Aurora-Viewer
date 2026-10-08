<#
.SYNOPSIS
    Range le viewer compilé et ses fichiers dans un dossier prêt à lancer
    (ou à zipper) : exe, assets, licences. La release GitHub et
    build-release.ps1 l'utilisent tous les deux : leurs dossiers sont
    identiques.

.EXAMPLE
    ./scripts/package.ps1 -Exe target/release/aurora-viewer.exe -Out Aurora-Viewer
#>
param(
    [Parameter(Mandatory)][string]$Exe,
    [Parameter(Mandatory)][string]$Out,
    # repository whose assets go with the exe (default: this script's)
    [string]$Source = (Split-Path $PSScriptRoot -Parent)
)

New-Item -ItemType Directory -Force -Path (Join-Path $Out 'assets\emoji') -ErrorAction Stop | Out-Null
Copy-Item -LiteralPath $Exe -Destination $Out -ErrorAction Stop
Copy-Item -Recurse -LiteralPath (Join-Path $Source 'crates\aurora-viewer\assets\character') -Destination (Join-Path $Out 'assets\character') -ErrorAction Stop
$emoji = 'Noto-3D-128.ttf', 'LICENSE-OFL.txt' | ForEach-Object { Join-Path $Source "assets\emoji\$_" }
Copy-Item -LiteralPath $emoji -Destination (Join-Path $Out 'assets\emoji') -ErrorAction Stop
$docs = 'LICENSE', 'NOTICE.md', 'README.md' | ForEach-Object { Join-Path $Source $_ }
Copy-Item -LiteralPath $docs -Destination $Out -ErrorAction Stop
