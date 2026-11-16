<#
.SYNOPSIS
    Prepares two test packages in .scratch, serves the next release on
    loopback and launches an isolated installation. No tag/release published.
    -Beta serves the next version as a nightly beta (a GitHub pre-release)
    and starts the isolated profile on the Beta channel: switching to Stable
    in Preferences › Language and updates hides the offer, Beta shows it.
#>
param([string]$Title = 'Test viewer-update', [Nullable[int]]$FpsLimit, [string]$Language = 'fr', [switch]$ClearCache, [switch]$ResetSettings, [switch]$Required, [switch]$Beta)
$ErrorActionPreference = 'Stop'
# UTF-8 without a BOM in Windows PowerShell 5.1 too, which has no
# Set-Content -Encoding utf8NoBOM.
function Write-Utf8 {
    param([Parameter(ValueFromPipeline = $true)][string]$Text, [Parameter(Mandatory = $true)][string]$LiteralPath)
    begin { $lines = @() }
    process { $lines += $Text }
    end { [IO.File]::WriteAllText($LiteralPath, (($lines -join "`r`n") + "`r`n"), (New-Object Text.UTF8Encoding $false)) }
}
$taskRepoRoot = (git rev-parse --show-toplevel).Trim()
Set-Location -LiteralPath $taskRepoRoot
if (!(Test-Path -LiteralPath (Join-Path $taskRepoRoot '.git') -PathType Leaf)) { throw 'Run this test from a dedicated worktree.' }
$taskRoot = Join-Path $taskRepoRoot ".scratch\update-test-$([Guid]::NewGuid().ToString('N'))"
$taskFeed = Join-Path $taskRoot 'feed'
New-Item -ItemType Directory -Path $taskFeed | Out-Null
$taskPreviousVersion = $env:AURORA_UPDATE_TEST_VERSION
$next = if ($Beta) { '0.2.101-beta.20261025.1' } else { '0.2.101' }
try {
    foreach ($version in @('0.2.100', $next)) {
        $env:AURORA_UPDATE_TEST_VERSION = "v$version"
        cargo build --locked -p aurora-viewer
        if ($LASTEXITCODE -ne 0) { throw "Build $version failed." }
        $out = if ($version -eq '0.2.100') { Join-Path $taskRoot 'install' } else { Join-Path $taskFeed "Aurora-Viewer-v$version-windows-x64" }
        & "$PSScriptRoot/package.ps1" -Exe (Join-Path $taskRepoRoot 'target/debug/aurora-viewer.exe') -Out $out
    }
} finally {
    if ($null -eq $taskPreviousVersion) { Remove-Item Env:AURORA_UPDATE_TEST_VERSION -ErrorAction SilentlyContinue } else { $env:AURORA_UPDATE_TEST_VERSION = $taskPreviousVersion }
}
# Restore the normal Git version in the worktree's ordinary build.
cargo build --locked -p aurora-viewer
if ($LASTEXITCODE -ne 0) { throw 'Failed to restore the normal build.' }
$name = "Aurora-Viewer-v$next-windows-x64.zip"
$zip = Join-Path $taskFeed $name
# Entries named with `/`: Compress-Archive of Windows PowerShell 5.1 writes
# `\`, which the updater refuses ("unsafe ZIP path").
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$taskPackage = (Resolve-Path -LiteralPath (Join-Path $taskFeed "Aurora-Viewer-v$next-windows-x64")).Path
$taskArchive = [IO.Compression.ZipFile]::Open($zip, [IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($file in Get-ChildItem -LiteralPath $taskPackage -Recurse -File) {
        $entry = (Split-Path -Leaf $taskPackage) + '/' + $file.FullName.Substring($taskPackage.Length + 1).Replace('\', '/')
        [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($taskArchive, $file.FullName, $entry)
    }
} finally {
    $taskArchive.Dispose()
}
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $name" | Write-Utf8 -LiteralPath "$zip.sha256"
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
$base = "http://127.0.0.1:$port"
@{ schema = 1; feed = "$base/releases.json" } | ConvertTo-Json | Write-Utf8 -LiteralPath (Join-Path $taskRoot 'aurora-update-test.json')
$taskPolicyPath = Join-Path $taskFeed 'aurora-update-policy.json'
# A real $null, not an empty `$(if …)`, which ConvertTo-Json of Windows
# PowerShell 5.1 writes as {} (the viewer then refuses the whole policy).
$taskMinimum = if ($Required) { '0.2.101' } else { $null }
$taskCache = if ($ClearCache) { '0.2.101' } else { $null }
$taskSettings = if ($ResetSettings) { '0.2.101' } else { $null }
@{ schema = 1; version = $next; minimum_supported_version = $taskMinimum; cache_invalidation_before = $taskCache; settings_reset_before = $taskSettings } | ConvertTo-Json | Write-Utf8 -LiteralPath $taskPolicyPath
New-Item -ItemType Directory -Path (Join-Path $taskRoot 'profile'), (Join-Path $taskRoot 'cache') | Out-Null
@{ settings_version = 9; language = $Language; draw_distance = 300; update_channel = $(if ($Beta) { 'Beta' } else { 'Stable' }) } | ConvertTo-Json | Write-Utf8 -LiteralPath (Join-Path $taskRoot 'profile/settings.json')
'Isolated cache preservation proof' | Set-Content -LiteralPath (Join-Path $taskRoot 'cache/update-marker.txt')
'Isolated profile preservation proof' | Set-Content -LiteralPath (Join-Path $taskRoot 'profile/update-marker.txt')
$assets = @(
    @{ name = $name; size = (Get-Item -LiteralPath $zip).Length; browser_download_url = "$base/$name" },
    @{ name = "$name.sha256"; size = (Get-Item -LiteralPath "$zip.sha256").Length; browser_download_url = "$base/$name.sha256" },
    @{ name = 'aurora-update-policy.json'; size = (Get-Item -LiteralPath $taskPolicyPath).Length; browser_download_url = "$base/aurora-update-policy.json" }
)
ConvertTo-Json -InputObject @(@{ tag_name = "v$next"; draft = $false; prerelease = [bool]$Beta; html_url = "$base/release.html"; body = "Trial of the real installation in an isolated folder.`nMaintenance policy announced before the restart."; assets = $assets }) -Depth 6 | Write-Utf8 -LiteralPath (Join-Path $taskFeed 'releases.json')
"<!doctype html><meta charset=`"utf-8`"><title>Test release</title><p>Local test release of Aurora $next.</p>" | Write-Utf8 -LiteralPath (Join-Path $taskFeed 'release.html')
$server = Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList @('-NoProfile', '-File', "`"$PSScriptRoot/update-test-server.ps1`"", '-Root', "`"$taskRoot`"", '-Port', $port) -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskRoot 'server.stdout.log') -RedirectStandardError (Join-Path $taskRoot 'server.stderr.log')
for ($i = 0; $i -lt 100 -and !(Test-Path -LiteralPath (Join-Path $taskRoot 'server-ready')); $i++) { if ($server.HasExited) { throw 'Test server stopped.' }; Start-Sleep -Milliseconds 100 }
if (!(Test-Path -LiteralPath (Join-Path $taskRoot 'server-ready'))) { throw 'Test server unavailable.' }
$taskPreviousRoot = $env:AURORA_UPDATE_TEST_ROOT
$taskPreviousDemo = $env:AURORA_DEMO
try {
    $env:AURORA_UPDATE_TEST_ROOT = $taskRoot
    Remove-Item Env:AURORA_DEMO -ErrorAction SilentlyContinue
    $arguments = @('--title', "`"$Title`"", '--language', $Language)
    if ($null -ne $FpsLimit) { $arguments += @('--fps-limit', $FpsLimit) }
    $viewer = Start-Process -FilePath (Join-Path $taskRoot 'install/aurora-viewer.exe') -WorkingDirectory (Join-Path $taskRoot 'install') -ArgumentList $arguments -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $taskRoot 'viewer.stdout.log') -RedirectStandardError (Join-Path $taskRoot 'viewer.stderr.log')
} finally {
    if ($null -eq $taskPreviousRoot) { Remove-Item Env:AURORA_UPDATE_TEST_ROOT -ErrorAction SilentlyContinue } else { $env:AURORA_UPDATE_TEST_ROOT = $taskPreviousRoot }
    if ($null -eq $taskPreviousDemo) { Remove-Item Env:AURORA_DEMO -ErrorAction SilentlyContinue } else { $env:AURORA_DEMO = $taskPreviousDemo }
}
@{ root = $taskRoot; server_pid = $server.Id; viewer_pid = $viewer.Id } | ConvertTo-Json | Write-Utf8 -LiteralPath (Join-Path $taskRepoRoot '.scratch/update-test-current.json')
Write-Output "Test installation: $taskRoot"
Write-Output "Viewer: $($viewer.Id); local server: $($server.Id)"
