<#
.SYNOPSIS
    Downloads the assets too heavy for git (the colour emoji font, 142 MB)
    into assets\emoji\. Without it, the viewer shows emojis without
    colour.
#>
param(
    [string]$Repo = 'Aurora-Viewer/Aurora-Viewer',
    [string]$Tag = 'assets-1'
)
. "$PSScriptRoot\common.ps1"

$files = @(
    @{ Name = 'Noto-3D-128.ttf'; Sha256 = '6F4312A7C02D0C9DE88095EE65E320FFE9316D2D8D56E4C515E7181FCA5ED439' }
)
$dest = Join-Path (Split-Path $PSScriptRoot -Parent) 'assets\emoji'
New-Item -ItemType Directory -Force -Path $dest | Out-Null

foreach ($f in $files) {
    $path = Join-Path $dest $f.Name
    if ((Test-Path $path) -and (Get-FileHash $path -Algorithm SHA256).Hash -eq $f.Sha256) {
        Write-Host "$($f.Name): already present."
        continue
    }
    $url = "https://github.com/$Repo/releases/download/$Tag/$($f.Name)"
    Write-Host "Downloading $url"
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest -Uri $url -OutFile "$path.part" -UseBasicParsing -ErrorAction Stop
    $hash = (Get-FileHash "$path.part" -Algorithm SHA256).Hash
    if ($hash -ne $f.Sha256) {
        Remove-Item "$path.part"
        throw "$($f.Name): unexpected hash ($hash)."
    }
    Move-Item -Force "$path.part" $path -ErrorAction Stop
    Write-Host "$($f.Name): OK." -ForegroundColor Green
}
