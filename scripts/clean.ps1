<#
.SYNOPSIS
    Montre la place prise par les compilations (dossiers target) et libère
    ce qui peut l'être. Par défaut, n'efface RIEN : il faut -Apply.

.DESCRIPTION
    Ne supprime que des dossiers de compilation Cargo (reconnus au fichier
    CACHEDIR.TAG que Cargo crée), et seulement à ces endroits :
      - dépôt principal : target\<profil> (debug, ci…), jamais target\release
        sauf avec -IncludeRelease ;
      - dossiers de tâche : work\<tâche>\target, seulement avec -Tasks (la
        tâche devra tout recompiler).
    Refuse de travailler pendant une compilation ou si un viewer tourne.

.EXAMPLE
    ./scripts/clean.ps1                 # affiche les tailles, n'efface rien
    ./scripts/clean.ps1 -Apply          # libère target\debug, target\ci… du dépôt
    ./scripts/clean.ps1 -Tasks -Apply   # libère aussi les target des tâches
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
        $items += @{ Path = $d.FullName; What = "dépôt : target\$($d.Name)"; Delete = -not $keep; Root = $mainTarget; Tag = $mainTarget }
    }
}
if (Test-Path -LiteralPath $p.Work) {
    foreach ($t in Get-ChildItem -LiteralPath $p.Work -Directory -Force) {
        $tt = Join-Path $t.FullName 'target'
        if (Test-Path -LiteralPath $tt) {
            $items += @{ Path = $tt; What = "tâche : $($t.Name)\target"; Delete = [bool]$Tasks; Root = $t.FullName; Tag = $tt }
        }
    }
}
if (-not $items) { Write-Host "Aucun dossier de compilation."; exit 0 }

$total = 0.0; $freed = 0.0
foreach ($i in $items) {
    $size = Get-SizeGB $i.Path
    $total += $size
    $action = if ($i.Delete) { 'à libérer' } else { 'gardé' }
    if ($i.Delete) { $freed += $size }
    Write-Host ("{0,8} Go  {1,-10}  {2}" -f $size, $action, $i.What)
}
Write-Host ("{0,8} Go  au total, {1} Go libérables" -f [math]::Round($total, 1), [math]::Round($freed, 1))

if (-not $Apply) {
    Write-Host "`nRien n'a été supprimé. Relance avec -Apply pour libérer la place (et -Tasks pour les dossiers de tâche)." -ForegroundColor Yellow
    exit 0
}

$busy = Get-Process cargo, rustc, clippy-driver, aurora-viewer -ErrorAction SilentlyContinue
if ($busy) {
    throw "Compilation ou viewer en cours ($(($busy.Name | Sort-Object -Unique) -join ', ')) : réessaie quand ils sont terminés."
}
foreach ($i in $items | Where-Object { $_.Delete }) {
    if (-not (Test-Deletable $i.Path $i.Tag $i.Root)) {
        Write-Warning "Ignoré (pas un dossier de compilation Cargo) : $($i.Path)"
        continue
    }
    Remove-Item -LiteralPath $i.Path -Recurse -Force -ErrorAction Continue
    if (Test-Path -LiteralPath $i.Path) {
        Write-Warning "Pas entièrement supprimé (fichier verrouillé ?) : $($i.Path)"
    } else {
        Write-Host "Libéré : $($i.What)" -ForegroundColor Green
    }
}
