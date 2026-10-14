<#
.SYNOPSIS
    Puts the media plugins in a folder ready to zip: the three Linden Lab
    binaries built from the Firestorm sources (SLPlugin.exe,
    media_plugin_cef.dll, media_plugin_libvlc.dll) and the CEF / Dullahan and
    VLC files of the prebuilt packages, laid out like the llplugin folder of
    Firestorm (viewer_manifest.py, Windows). The "Media plugins" workflow
    runs it after the build.

.EXAMPLE
    ./scripts/package-media-plugins.ps1 -Source fs -Build fs/build-vc170-64 -Out Aurora-Media-Plugins-1-windows-x64 -Version 1
#>
param(
    # checkout of the Firestorm sources, at the commit that was built
    [Parameter(Mandatory)][string]$Source,
    # its CMake build folder (holds packages\ and the built targets)
    [Parameter(Mandatory)][string]$Build,
    [Parameter(Mandatory)][string]$Out,
    [Parameter(Mandatory)][string]$Version,
    [string]$Configuration = 'Release'
)
$ErrorActionPreference = 'Stop'

$packages = Join-Path $Build 'packages'
$bin = Join-Path $packages 'bin\release'
$resources = Join-Path $packages 'resources'
$plugin = Join-Path $Out 'llplugin'
New-Item -ItemType Directory -Force -Path $plugin | Out-Null

function Copy-Required([string]$From, [string[]]$Names, [string]$To) {
    foreach ($name in $Names) {
        $path = Join-Path $From $name
        if (-not (Test-Path -LiteralPath $path)) { throw "Missing from the build: $path" }
        Copy-Item -LiteralPath $path -Destination $To -Recurse
    }
}

# the launcher sits with the plugins: it finds the C++ runtime next to it
Copy-Required (Join-Path $Build "llplugin\slplugin\$Configuration") 'SLPlugin.exe' $plugin
Copy-Required (Join-Path $Build "media_plugins\cef\$Configuration") 'media_plugin_cef.dll' $plugin
Copy-Required (Join-Path $Build "media_plugins\libvlc\$Configuration") 'media_plugin_libvlc.dll' $plugin

# CEF runtime and the Dullahan host process
Copy-Required $bin @(
    'chrome_elf.dll', 'd3dcompiler_47.dll', 'dxcompiler.dll', 'dxil.dll', 'libcef.dll', 'libEGL.dll',
    'libGLESv2.dll', 'v8_context_snapshot.bin', 'vk_swiftshader.dll', 'vk_swiftshader_icd.json',
    'vulkan-1.dll', 'dullahan_host.exe'
) $plugin
Copy-Required $resources @('chrome_100_percent.pak', 'chrome_200_percent.pak', 'resources.pak', 'icudtl.dat', 'locales') $plugin

# LibVLC and its modules
Copy-Required $bin @('libvlc.dll', 'libvlccore.dll', 'plugins') $plugin

# C++ runtime of the compiler that built the three binaries
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw 'Visual Studio with the C++ tools was not found.' }
$crt = Get-ChildItem -Path (Join-Path $vs 'VC\Redist\MSVC') -Recurse -Directory -Filter 'Microsoft.VC*.CRT' |
    Where-Object { $_.FullName -match '\\x64\\' -and $_.FullName -notmatch 'onecore|debug_nonredist' } |
    Sort-Object FullName -Descending | Select-Object -First 1
if (-not $crt) { throw 'The Visual C++ runtime to redistribute was not found.' }
Copy-Required $crt.FullName @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') $plugin

# licence texts of the packages, next to the binaries they cover
$licenses = Join-Path $Out 'LICENSES'
New-Item -ItemType Directory -Force -Path $licenses | Out-Null
Copy-Item -Path (Join-Path $packages 'LICENSES\*') -Destination $licenses -Recurse
Copy-Item -LiteralPath (Join-Path $Source 'LICENSE') -Destination (Join-Path $licenses 'firestorm-viewer-LGPL-2.1.txt')

# what was built, for the viewer (version check) and for the licence notice
function Get-PackageVersion([string]$Name) {
    $xml = [IO.File]::ReadAllText((Join-Path (Resolve-Path -LiteralPath $Source).Path 'autobuild.xml'))
    $match = [regex]::Match($xml, "<key>version</key>\s*<string>([^<]+)</string>\s*<key>name</key>\s*<string>$([regex]::Escape($Name))</string>")
    if (-not $match.Success) { throw "No version of $Name in autobuild.xml." }
    $match.Groups[1].Value
}
$manifest = [ordered]@{
    schema   = 1
    version  = $Version
    source   = [ordered]@{
        repository = (git -C $Source remote get-url origin).Trim()
        commit     = (git -C $Source rev-parse HEAD).Trim()
    }
    packages = [ordered]@{
        dullahan = Get-PackageVersion 'dullahan'
        'vlc-bin' = Get-PackageVersion 'vlc-bin'
    }
}
$manifestPath = Join-Path (Resolve-Path -LiteralPath $Out).Path 'aurora-media-plugins.json'
# Windows PowerShell 5.1 does not support Set-Content -Encoding utf8NoBOM.
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json), [Text.UTF8Encoding]::new($false))

$files = Get-ChildItem -LiteralPath $Out -Recurse -File
Write-Host ("{0}: {1} files, {2:N1} MB" -f $Out, $files.Count, (($files | Measure-Object Length -Sum).Sum / 1MB))
