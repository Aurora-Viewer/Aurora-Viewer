<#
.SYNOPSIS
    Prepares two test packages in .scratch, serves the next release on
    loopback and launches an isolated installation. No tag/release published.
#>
param([string]$Title = 'Test viewer-update', [Nullable[int]]$FpsLimit, [string]$Language = 'fr', [switch]$ClearCache, [switch]$ResetSettings, [switch]$Required)
$ErrorActionPreference = 'Stop'
$taskRepoRoot = (git rev-parse --show-toplevel).Trim()
Set-Location -LiteralPath $taskRepoRoot
if (!(Test-Path -LiteralPath (Join-Path $taskRepoRoot '.git') -PathType Leaf)) { throw 'Run this test from a dedicated worktree.' }
$taskRoot = Join-Path $taskRepoRoot ".scratch\update-test-$([Guid]::NewGuid().ToString('N'))"
$taskFeed = Join-Path $taskRoot 'feed'
New-Item -ItemType Directory -Path $taskFeed | Out-Null
$taskPreviousVersion = $env:AURORA_UPDATE_TEST_VERSION
try {
    foreach ($version in @('0.2.100', '0.2.101')) {
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
$name = 'Aurora-Viewer-v0.2.101-windows-x64.zip'
$zip = Join-Path $taskFeed $name
Compress-Archive -LiteralPath (Join-Path $taskFeed 'Aurora-Viewer-v0.2.101-windows-x64') -DestinationPath $zip
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $name" | Set-Content -LiteralPath "$zip.sha256" -Encoding utf8NoBOM
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
$base = "http://127.0.0.1:$port"
@{ schema = 1; feed = "$base/releases.json" } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRoot 'aurora-update-test.json') -Encoding utf8NoBOM
$taskPolicyPath = Join-Path $taskFeed 'aurora-update-policy.json'
@{ schema = 1; version = '0.2.101'; minimum_supported_version = $(if ($Required) { '0.2.101' }); cache_invalidation_before = $(if ($ClearCache) { '0.2.101' }); settings_reset_before = $(if ($ResetSettings) { '0.2.101' }) } | ConvertTo-Json | Set-Content -LiteralPath $taskPolicyPath -Encoding utf8NoBOM
New-Item -ItemType Directory -Path (Join-Path $taskRoot 'profile'), (Join-Path $taskRoot 'cache') | Out-Null
@{ settings_version = 9; language = $Language; draw_distance = 300 } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRoot 'profile/settings.json') -Encoding utf8NoBOM
'Isolated cache preservation proof' | Set-Content -LiteralPath (Join-Path $taskRoot 'cache/update-marker.txt')
'Isolated profile preservation proof' | Set-Content -LiteralPath (Join-Path $taskRoot 'profile/update-marker.txt')
$assets = @(
    @{ name = $name; size = (Get-Item -LiteralPath $zip).Length; browser_download_url = "$base/$name" },
    @{ name = "$name.sha256"; size = (Get-Item -LiteralPath "$zip.sha256").Length; browser_download_url = "$base/$name.sha256" },
    @{ name = 'aurora-update-policy.json'; size = (Get-Item -LiteralPath $taskPolicyPath).Length; browser_download_url = "$base/aurora-update-policy.json" }
)
ConvertTo-Json -InputObject @(@{ tag_name = 'v0.2.101'; draft = $false; prerelease = $false; html_url = "$base/release.html"; body = "Trial of the real installation in an isolated folder.`nMaintenance policy announced before the restart."; assets = $assets }) -Depth 6 | Set-Content -LiteralPath (Join-Path $taskFeed 'releases.json') -Encoding utf8NoBOM
'<!doctype html><meta charset="utf-8"><title>Test release</title><p>Local test release of Aurora 0.2.101.</p>' | Set-Content -LiteralPath (Join-Path $taskFeed 'release.html') -Encoding utf8NoBOM
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
@{ root = $taskRoot; server_pid = $server.Id; viewer_pid = $viewer.Id } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRepoRoot '.scratch/update-test-current.json') -Encoding utf8NoBOM
Write-Output "Test installation: $taskRoot"
Write-Output "Viewer: $($viewer.Id); local server: $($server.Id)"
