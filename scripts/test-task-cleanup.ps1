<#
.SYNOPSIS
    Tests the last build date of a task's target\ (Get-LastBuildTime and
    Set-LastBuildMarker, common.ps1), which the housekeeping of new-task.ps1
    and the task list of aurora-tools.cmd read. Fixture folders only, in
    .scratch\: no real task, build or setting is touched.
#>
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\common.ps1"
$ErrorActionPreference = 'Stop'

function Assert-CleanupTest([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

# Creates the folders (and files, for names with an extension) of a fake
# target, then dates each entry: children first, since creating an entry
# changes the date of its folder.
function New-FakeTarget([string]$Root, [hashtable]$Ages) {
    New-Item -ItemType Directory -Force -Path $Root | Out-Null
    [IO.File]::WriteAllText((Join-Path $Root 'CACHEDIR.TAG'), "Signature: 8a477f597d28d172789f06886806bc55`n")
    foreach ($rel in $Ages.Keys) {
        $path = Join-Path $Root $rel
        if ([IO.Path]::GetExtension($rel)) {
            New-Item -ItemType Directory -Force -Path (Split-Path $path -Parent) | Out-Null
            [IO.File]::WriteAllText($path, 'fixture')
        } else {
            New-Item -ItemType Directory -Force -Path $path | Out-Null
        }
    }
    foreach ($rel in $Ages.Keys | Sort-Object { $_.Split('\').Count } -Descending) {
        $path = Join-Path $Root $rel
        $when = (Get-Date).AddHours(-$Ages[$rel])
        if (Test-Path -LiteralPath $path -PathType Container) { [IO.Directory]::SetLastWriteTime($path, $when) }
        else { [IO.File]::SetLastWriteTime($path, $when) }
    }
}

function Get-AgeHours([string]$Target) {
    $last = Get-LastBuildTime $Target
    if ($null -eq $last) { return $null }
    ((Get-Date) - $last).TotalHours
}

function Test-Near($Hours, [double]$Expected) { $null -ne $Hours -and [math]::Abs($Hours - $Expected) -lt 0.1 }

$scratch = Join-Path (Split-Path $PSScriptRoot -Parent) '.scratch'
$fixture = Join-Path $scratch "cleanup-test-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $fixture -Force | Out-Null
try {
    # Cargo 1.100: no deps\, the crates are in build\<crate>\<hash>\. A task
    # idle for 30 hours, whose crates were first built a month ago.
    $new = Join-Path $fixture 'new-layout'
    New-FakeTarget $new @{
        'debug\build\serde\0123abcd' = 720; 'debug\build\serde' = 720; 'debug\build\aurora-viewer\4567ef01' = 700
        'debug\build\aurora-viewer' = 700; 'debug\build' = 700; 'debug\incremental\aurora_viewer-1' = 30
        'debug\incremental' = 30; 'debug\.cargo-lock' = 720; 'debug' = 30
    }
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 30) 'New layout: the last build is the incremental folder.'

    # The trim of new-task.ps1: marker first, then the incremental caches go.
    # Deleting them redates debug\ to now; the age must stay 30 hours, and
    # above all must not become unknown (the 2026-11-22 bug: "infinitely"
    # idle, whole target deleted at the next run).
    Set-LastBuildMarker $new (Get-LastBuildTime $new)
    foreach ($d in Get-IncrementalDirs $new) { Remove-Item -LiteralPath $d -Recurse -Force }
    Assert-CleanupTest (-not (Get-IncrementalDirs $new)) 'The fixture trim must remove incremental\.'
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 30) 'After a trim, the age must still count from the last build.'

    # Without the marker, the per-crate folders still date it (no unknown).
    Remove-Item -LiteralPath (Join-Path $new $LastBuildMarker)
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 700) 'Without incremental\ nor marker, the build\ folders date the target.'

    # A build after the trim is newer than the marker.
    Set-LastBuildMarker $new (Get-Date).AddHours(-30)
    New-FakeTarget $new @{ 'debug\incremental\aurora_viewer-2' = 1; 'debug\incremental' = 1 }
    Assert-CleanupTest (Test-Near (Get-AgeHours $new) 1) 'A build after the trim must win over the marker.'

    # Older Cargo: deps\ and .fingerprint\; and a release build, without
    # incremental\, dated by the exe Cargo writes into the profile folder.
    $old = Join-Path $fixture 'old-layout'
    New-FakeTarget $old @{ 'debug\deps' = 5; 'debug\.fingerprint' = 6; 'debug\incremental' = 9; 'release\deps' = 50; 'release\aurora-viewer.exe' = 3 }
    Assert-CleanupTest (Test-Near (Get-AgeHours $old) 3) 'Older layout: deps\, .fingerprint\ and the profile files date the target.'

    # Nothing to date it: unknown, never "infinitely" old.
    $empty = Join-Path $fixture 'empty'
    New-FakeTarget $empty @{}
    Assert-CleanupTest ($null -eq (Get-LastBuildTime $empty)) 'A target with no build entry has no date.'
    Assert-CleanupTest ($null -eq (Get-LastBuildTime (Join-Path $fixture 'missing'))) 'A missing target has no date.'

    Write-Output 'Task cleanup: new and older Cargo layouts, trim marker, later build and unknown date passed.'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    if ((Split-Path $resolved -Parent) -ne [IO.Path]::GetFullPath($scratch) -or (Split-Path $resolved -Leaf) -notlike 'cleanup-test-*') { throw 'Cleanup outside fixture refused.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
