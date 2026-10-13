<#
.SYNOPSIS
    Shows the space taken by builds (target folders) and frees what can be
    freed. By default, deletes NOTHING: -Apply is required.

.DESCRIPTION
    Only deletes Cargo build folders (recognised by the CACHEDIR.TAG file
    that Cargo creates), and only in these places:
      - main repository: target\<profile> (debug, ci…), never target\release
        except with -IncludeRelease;
      - task folders: work\<task>\target, only with -Tasks (the task will
        have to rebuild everything);
      - RELEASE\src\target (build-release.ps1), only with -IncludeRelease
        (the next release build will start from scratch).
    Refuses to work during a build or while a viewer is running.

.EXAMPLE
    ./scripts/clean.ps1                 # shows the sizes, deletes nothing
    ./scripts/clean.ps1 -Apply          # frees target\debug, target\ci… of the repository
    ./scripts/clean.ps1 -Tasks -Apply   # also frees the target folders of the tasks
#>
param(
    [switch]$Apply,
    [switch]$Tasks,
    [switch]$IncludeRelease
)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths

function Get-SizeGB([string]$Path) {
    $sum = (Get-ChildItem -LiteralPath $Path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    [math]::Round(($sum / 1GB), 1)
}

# Safe to delete: $Path is strictly inside $Root, and $TagDir (the Cargo
# target directory it belongs to) holds the CACHEDIR.TAG Cargo creates.
function Test-Deletable([string]$Path, [string]$TagDir, [string]$Root) {
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\')
    $root = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
    $full.StartsWith($root, [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath (Join-Path $TagDir 'CACHEDIR.TAG'))
}

# candidates: @{ Path; What; Delete }
$items = @()
$mainTarget = Join-Path $p.Main 'target'
if (Test-Path -LiteralPath $mainTarget) {
    foreach ($d in Get-ChildItem -LiteralPath $mainTarget -Directory -Force) {
        $keep = ($d.Name -eq 'release') -and -not $IncludeRelease
        $items += @{ Path = $d.FullName; What = "repository: target\$($d.Name)"; Delete = -not $keep; Root = $mainTarget; Tag = $mainTarget }
    }
}
if (Test-Path -LiteralPath $p.Work) {
    foreach ($t in Get-ChildItem -LiteralPath $p.Work -Directory -Force) {
        $tt = Join-Path $t.FullName 'target'
        if (Test-Path -LiteralPath $tt) {
            $items += @{ Path = $tt; What = "task: $($t.Name)\target"; Delete = [bool]$Tasks; Root = $t.FullName; Tag = $tt }
        }
    }
}
$relSrc = Join-Path $p.Root 'RELEASE\src'
$relTarget = Join-Path $relSrc 'target'
if (Test-Path -LiteralPath $relTarget) {
    $items += @{ Path = $relTarget; What = 'RELEASE\src\target'; Delete = [bool]$IncludeRelease; Root = $relSrc; Tag = $relTarget }
}
if (-not $items) { Write-Host "No build folder."; exit 0 }

$total = 0.0; $freed = 0.0
foreach ($i in $items) {
    $size = Get-SizeGB $i.Path
    $total += $size
    $action = if ($i.Delete) { 'to free' } else { 'kept' }
    if ($i.Delete) { $freed += $size }
    Write-Host ("{0,8} GB  {1,-10}  {2}" -f $size, $action, $i.What)
}
Write-Host ("{0,8} GB  in total, {1} GB can be freed" -f [math]::Round($total, 1), [math]::Round($freed, 1))

if (-not $Apply) {
    Write-Host "`nNothing was deleted. Run again with -Apply to free the space (and -Tasks for the task folders)." -ForegroundColor Yellow
    exit 0
}

$busy = Get-Process cargo, rustc, clippy-driver, aurora-viewer -ErrorAction SilentlyContinue
if ($busy) {
    throw "A build or a viewer is running ($(($busy.Name | Sort-Object -Unique) -join ', ')): try again when they have finished."
}
foreach ($i in $items | Where-Object { $_.Delete }) {
    if (-not (Test-Deletable $i.Path $i.Tag $i.Root)) {
        Write-Warning "Skipped (not a Cargo build folder): $($i.Path)"
        continue
    }
    Remove-Item -LiteralPath $i.Path -Recurse -Force -ErrorAction Continue
    if (Test-Path -LiteralPath $i.Path) {
        Write-Warning "Not fully deleted (locked file?): $($i.Path)"
    } else {
        Write-Host "Freed: $($i.What)" -ForegroundColor Green
    }
}
