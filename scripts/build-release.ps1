<#
.SYNOPSIS
    Compile le dernier main de GitHub (ou une PR) en --release, à côté du
    dépôt : ..\RELEASE\Aurora-Viewer\aurora-viewer.exe, prêt à lancer.

.DESCRIPTION
    Le code compilé vient toujours de GitHub, jamais du dépôt local : ni les
    changements non commités ni les commits non poussés n'y entrent.
    ..\RELEASE\src est un worktree réservé, remis à chaque fois sur le commit
    demandé ; son target\ est gardé d'un build à l'autre (seul ce qui a
    changé est recompilé). version.txt dit quel commit a été compilé.
    Le premier lancement crée ..\RELEASE\build-release.bat (double-clic).

.EXAMPLE
    ./scripts/build-release.ps1          # dernier main de GitHub
    ./scripts/build-release.ps1 -Pr 42   # la PR n°42
#>
param([int]$Pr)
. "$PSScriptRoot\common.ps1"

$p = Get-AuroraPaths
$rel = Join-Path $p.Root 'RELEASE'
$src = Join-Path $rel 'src'
$out = Join-Path $rel 'Aurora-Viewer'
New-Item -ItemType Directory -Force -Path $rel -ErrorAction Stop | Out-Null

# Double-click launcher (build-release.bat 42 builds PR 42). It runs the
# script of the last build, so it is always main's latest version; when that
# build lacks it (a PR branched before it existed), it first puts src back on
# GitHub's main, and only then falls back to the repository's copy.
# Only written once: cmd reads a .bat from disk while running it.
$bat = Join-Path $rel 'build-release.bat'
if (-not (Test-Path -LiteralPath $bat)) {
    @(
        '@echo off'
        'set "S=%~dp0src\scripts\build-release.ps1"'
        'if not exist "%S%" (git -C "%~dp0src" fetch origin main --quiet && git -C "%~dp0src" checkout --detach --force --quiet origin/main)'
        "if not exist `"%S%`" set `"S=$(Join-Path $p.Main 'scripts\build-release.ps1')`""
        'powershell -NoProfile -ExecutionPolicy Bypass -File "%S%" %*'
        'pause'
    ) | Set-Content -LiteralPath $bat -Encoding ASCII -ErrorAction Stop
}

if (Test-InUse $out) { throw "Le viewer de $out est ouvert : ferme-le, puis relance." }

# the commit to build, straight from GitHub
if ($Pr) {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin "pull/$Pr/head" --quiet }
    $sha = (git -C $p.Main rev-parse FETCH_HEAD).Trim()
    $what = "PR #$Pr"
} else {
    Invoke-Checked 'git fetch' { git -C $p.Main fetch origin main --quiet }
    $sha = (git -C $p.Main rev-parse origin/main).Trim()
    $what = 'main'
}

# reset the reserved worktree on it; target\ is ignored by git, so kept
if (Test-Path -LiteralPath (Join-Path $src '.git')) {
    Invoke-Checked 'git checkout' { git -C $src checkout --detach --force --quiet $sha }
    Invoke-Checked 'git clean' { git -C $src clean -fd --quiet }
} else {
    Invoke-Checked 'git worktree add' { git -C $p.Main worktree add --detach $src $sha }
}

# the emoji font is not in git: hard link it from the main repository, or download it
$font = 'assets\emoji\Noto-3D-128.ttf'
if (-not (Test-Path -LiteralPath (Join-Path $src $font))) {
    try {
        New-Item -ItemType HardLink -Path (Join-Path $src $font) -Target (Join-Path $p.Main $font) -ErrorAction Stop | Out-Null
    } catch {
        & (Join-Path $src 'scripts\fetch-assets.ps1')
    }
}

Write-Host "Compilation de $what ($($sha.Substring(0, 9))) en --release…" -ForegroundColor Cyan
$sw = [Diagnostics.Stopwatch]::StartNew()
Push-Location $src
try {
    Invoke-Checked 'cargo build' { cargo build --release --locked -p aurora-viewer }
} finally {
    Pop-Location
}

# package.ps1 of the built commit when it has one (older PR branches do not)
$package = Join-Path $src 'scripts\package.ps1'
if (-not (Test-Path -LiteralPath $package)) { $package = Join-Path $PSScriptRoot 'package.ps1' }
if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force -ErrorAction Stop }
& $package -Exe (Join-Path $src 'target\release\aurora-viewer.exe') -Out $out -Source $src
@(
    "$what : $((git -C $src log -1 --format='%h %s (%ci)').Trim())"
    "compilé le $(Get-Date -Format 'dd/MM/yyyy HH:mm')"
) | Set-Content -LiteralPath (Join-Path $out 'version.txt') -Encoding UTF8 -ErrorAction Stop

Write-Host ""
Write-Host "Prêt en $([math]::Floor($sw.Elapsed.TotalMinutes)) min $($sw.Elapsed.Seconds) s : $out\aurora-viewer.exe" -ForegroundColor Green
Write-Host "Double-clic sur $bat pour recompiler (build-release.bat 42 : la PR n°42)."
